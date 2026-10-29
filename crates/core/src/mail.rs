//! Resend-compatible delivery with an idempotent outbox and remote delivery confirmation.
use crate::Db;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn configured() -> bool {
    ["MAIL_API_KEY", "MAIL_FROM", "MAIL_TO"].iter().all(|key| {
        std::env::var(key)
            .ok()
            .is_some_and(|v| !v.trim().is_empty())
    })
}

/// The provider endpoint and the one sender and recipient. Production reads it from the environment;
/// tests point it at a loopback server.
pub struct Transport {
    pub base: String,
    pub key: String,
    pub from: String,
    pub to: String,
}

pub async fn dispatch(db: &Db) -> anyhow::Result<()> {
    if !configured() {
        return Ok(());
    }
    let transport = Transport {
        base: std::env::var("MAIL_API_URL").unwrap_or_else(|_| "https://api.resend.com".into()),
        key: std::env::var("MAIL_API_KEY")?,
        from: std::env::var("MAIL_FROM")?,
        to: std::env::var("MAIL_TO")?,
    };
    deliver(db, &transport).await
}

/// Send what is due and confirm what was accepted. Only 'pending' and 'accepted' rows are touched;
/// any other state, including a legacy row suppressed at cutover, is never emailed.
pub async fn deliver(db: &Db, transport: &Transport) -> anyhow::Result<()> {
    let Transport {
        base,
        key,
        from,
        to,
    } = transport;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    // A provider's idempotency window is finite. Never retry an ambiguous old send.
    sqlx::query("update notifications set delivery_state='failed',delivery_error='Delivery could not be confirmed; automatic retry stopped to prevent duplicate email.' where delivery_receipt is null and first_attempt_at<now()-interval '23 hours' and delivered_at is null")
        .execute(db).await?;
    let mut tx = db.begin().await?;
    let rows:Vec<(Uuid,String,Value,Option<String>)>=sqlx::query_as("select n.id,n.kind,jsonb_build_object('employer',coalesce(org.display_name,n.payload->>'source'),'position',coalesce(o.canonical_title,'career site'),'url',coalesce((select p.url from postings p where p.opportunity_id=o.id order by p.url limit 1),''),'time',n.created_at,'detail',n.payload),n.delivery_receipt from notifications n left join applications a on a.id=n.application_id left join opportunities o on o.id=a.opportunity_id left join organizations org on org.id=o.organization_id where n.kind in ('submitted','action_required','submission_uncertain','failed','manual_required','source_failing','new_matches','deadline_reminder') and n.delivered_at is null and n.delivery_state in ('pending','accepted') and (n.last_attempt_at is null or n.last_attempt_at<now()-interval '1 minute') order by n.created_at for update of n skip locked limit 5").fetch_all(&mut *tx).await?;
    // Persist the attempt before network I/O: a process crash must not reset the retry window.
    for (id, _, _, _) in &rows {
        sqlx::query("update notifications set first_attempt_at=coalesce(first_attempt_at,now()),last_attempt_at=now() where id=$1")
            .bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    for (id, kind, info, receipt) in rows {
        let outcome: anyhow::Result<(String, Option<String>)> = async {
            if let Some(receipt) = receipt {
                let response = client
                    .get(format!("{base}/emails/{receipt}"))
                    .bearer_auth(key)
                    .send()
                    .await?
                    .error_for_status()?
                    .json::<Value>()
                    .await?;
                let state = match response["last_event"].as_str() {
                    Some("delivered") => "delivered",
                    Some("bounced" | "failed") => "bounced",
                    _ => "accepted",
                };
                return Ok((state.into(), Some(receipt)));
            }
            let (subject, text) = compose(&kind, &info);
            let body = json!({"from":from,"to":[to],"subject":subject,"text":text});
            let response = client
                .post(format!("{base}/emails"))
                .bearer_auth(key)
                .header("Idempotency-Key", id.to_string())
                .json(&body)
                .send()
                .await?
                .error_for_status()?
                .json::<Value>()
                .await?;
            let receipt = response["id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Email provider returned no message identifier"))?;
            // Acceptance is not delivery. A later status read must confirm delivered.
            Ok(("accepted".into(), Some(receipt.into())))
        }
        .await;
        match outcome {
            Ok((state, receipt)) => {
                sqlx::query("update notifications set delivery_state=$2,delivery_receipt=$3,delivery_error=null,last_attempt_at=now(),delivered_at=case when $2='delivered' then now() else null end where id=$1").bind(id).bind(state).bind(receipt).execute(db).await?;
            }
            Err(error) => {
                sqlx::query(
                    "update notifications set delivery_error=$2,last_attempt_at=now() where id=$1",
                )
                .bind(id)
                .bind(error.to_string())
                .execute(db)
                .await?;
            }
        }
    }
    Ok(())
}

/// Subject and plain-text body of one notification. Digests and reminders carry their own items in
/// the payload; everything else describes one application. Nothing personal beyond the role and the
/// employer goes into an email.
fn compose(kind: &str, info: &Value) -> (String, String) {
    let text = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let detail = &info["detail"];
    let (status, hint) = wording(kind);
    match kind {
        "new_matches" => {
            let items = detail["items"].as_array().cloned().unwrap_or_default();
            let mut body = format!("{status} for {}.\n", text(&detail["profile"]));
            for item in &items {
                body.push_str(&format!(
                    "\n{} · {}\n  {} · {} · priority {}\n  {}\n",
                    text(&item["title"]),
                    text(&item["employer"]),
                    text(&item["stage"]),
                    text(&item["location"]),
                    item["priority"],
                    text(&item["explanation"])
                ));
                if let Some(gap) = item["gap"].as_str() {
                    body.push_str(&format!("  Check: {gap}\n"));
                }
                if let Some(deadline) = item["deadline"].as_str() {
                    body.push_str(&format!("  Deadline: {deadline}\n"));
                }
                body.push_str(&format!("  {}\n", text(&item["url"])));
            }
            body.push_str(&format!("\n{hint}"));
            (
                format!(
                    "{status}: {} new for {}",
                    items.len(),
                    text(&detail["profile"])
                ),
                body,
            )
        }
        "deadline_reminder" => (
            format!(
                "{status}: {} at {} in {} day(s)",
                text(&detail["title"]),
                text(&detail["employer"]),
                detail["days_left"]
            ),
            format!(
                "{status}\nPosition: {}\nEmployer: {}\nDeadline: {}\nPosting: {}\n{hint}",
                text(&detail["title"]),
                text(&detail["employer"]),
                text(&detail["deadline"]),
                text(&detail["url"])
            ),
        ),
        _ => {
            let (employer, position) = (text(&info["employer"]), text(&info["position"]));
            (
                format!("{status}: {position} at {employer}"),
                format!("{status}\nEmployer: {employer}\nPosition: {position}\nApplication: {}\nTime: {}\n{hint}", text(&info["url"]), text(&info["time"])),
            )
        }
    }
}

/// Subject status and next step per notification kind. Anything that needs the owner says what to do.
fn wording(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "submitted" => ("Confirmed submitted", "The employer confirmed receipt."),
        "submission_uncertain" => ("Check with the employer", "The form may have been sent, but no confirmation was seen. It will not be retried; check with the employer before applying again."),
        "failed" => ("Application failed", "Nothing was confirmed as sent. Open Opportunities to retry it."),
        "manual_required" => ("Complete on the employer website", "Automation cannot finish this form. Open Opportunities for the employer link."),
        "source_failing" => ("Source not refreshing", "Its listings are kept, not closed, while it fails. Check whether the employer moved its career site."),
        "new_matches" => ("New matches", "Priority is a ranking for you, not a hiring probability. Open Opportunities to save, dismiss or apply."),
        "deadline_reminder" => ("Deadline coming up", "Open Opportunities to apply or dismiss it; no reminder is sent once you have applied."),
        _ => ("Your answer is needed", "Open Opportunities to answer the employer's questions."),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_kind_that_needs_the_owner_has_its_own_subject() {
        let subjects: std::collections::HashSet<_> = [
            "submitted",
            "action_required",
            "submission_uncertain",
            "failed",
            "manual_required",
            "source_failing",
            "new_matches",
            "deadline_reminder",
        ]
        .iter()
        .map(|k| super::wording(k).0)
        .collect();
        assert_eq!(subjects.len(), 8);
    }

    #[test]
    fn a_digest_lists_each_match_with_its_reason_and_never_claims_a_hiring_chance() {
        let info = serde_json::json!({"detail": {"profile": "Firmware and platform systems", "items": [
            {"title": "Firmware Intern", "employer": "Example Silicon", "stage": "Internship", "location": "Munich, DE",
             "priority": 84, "explanation": "Your CV: C, RTOS", "gap": null, "deadline": "2026-11-30", "url": "https://jobs.lever.co/example/1"},
            {"title": "RTL Engineer", "employer": "Example Chips", "stage": "Level not stated", "location": "DE",
             "priority": 61, "explanation": "Your CV: SystemVerilog", "gap": "3+ years required; not established by your CV", "deadline": null, "url": "https://jobs.lever.co/example/2"}]}});
        let (subject, body) = super::compose("new_matches", &info);
        assert_eq!(
            subject,
            "New matches: 2 new for Firmware and platform systems"
        );
        for expected in [
            "Firmware Intern · Example Silicon",
            "Internship · Munich, DE · priority 84",
            "Deadline: 2026-11-30",
            "Check: 3+ years required",
            "https://jobs.lever.co/example/2",
            "not a hiring probability",
        ] {
            assert!(body.contains(expected), "missing {expected:?} in\n{body}");
        }
        let reminder = serde_json::json!({"detail": {"title": "Masterarbeit RISC-V", "employer": "Example Institute", "deadline": "2026-11-02", "days_left": 3, "url": "https://example.test/thesis"}});
        let (subject, body) = super::compose("deadline_reminder", &reminder);
        assert_eq!(
            subject,
            "Deadline coming up: Masterarbeit RISC-V at Example Institute in 3 day(s)"
        );
        assert!(body.contains("Deadline: 2026-11-02"));
    }
}
