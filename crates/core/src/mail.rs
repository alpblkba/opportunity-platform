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

pub async fn dispatch(db: &Db) -> anyhow::Result<()> {
    if !configured() {
        return Ok(());
    }
    let base = std::env::var("MAIL_API_URL").unwrap_or_else(|_| "https://api.resend.com".into());
    let key = std::env::var("MAIL_API_KEY")?;
    let from = std::env::var("MAIL_FROM")?;
    let to = std::env::var("MAIL_TO")?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    // A provider's idempotency window is finite. Never retry an ambiguous old send.
    sqlx::query("update notifications set delivery_state='failed',delivery_error='Delivery could not be confirmed; automatic retry stopped to prevent duplicate email.' where delivery_receipt is null and first_attempt_at<now()-interval '23 hours' and delivered_at is null")
        .execute(db).await?;
    let mut tx = db.begin().await?;
    let rows:Vec<(Uuid,String,Value,Option<String>)>=sqlx::query_as("select n.id,n.kind,jsonb_build_object('employer',coalesce(org.display_name,n.payload->>'source'),'position',coalesce(o.canonical_title,'career site'),'url',coalesce((select p.url from postings p where p.opportunity_id=o.id order by p.url limit 1),''),'time',n.created_at,'detail',n.payload),n.delivery_receipt from notifications n left join applications a on a.id=n.application_id left join opportunities o on o.id=a.opportunity_id left join organizations org on org.id=o.organization_id where n.kind in ('submitted','action_required','submission_uncertain','failed','manual_required','source_failing') and n.delivered_at is null and n.delivery_state not in ('bounced','failed') and (n.last_attempt_at is null or n.last_attempt_at<now()-interval '1 minute') order by n.created_at for update of n skip locked limit 5").fetch_all(&mut *tx).await?;
    // Persist the attempt before network I/O: a process crash must not reset the retry window.
    for (id, _, _, _) in &rows {
        sqlx::query("update notifications set first_attempt_at=coalesce(first_attempt_at,now()),last_attempt_at=now() where id=$1")
            .bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    for (id, kind, info, receipt) in rows {
        let outcome: anyhow::Result<(String,Option<String>)>=async {
            if let Some(receipt)=receipt {
                let response=client.get(format!("{base}/emails/{receipt}")).bearer_auth(&key).send().await?.error_for_status()?.json::<Value>().await?;
                let state=match response["last_event"].as_str(){Some("delivered")=>"delivered",Some("bounced"|"failed")=>"bounced",_=>"accepted"};
                return Ok((state.into(),Some(receipt)));
            }
            let employer=info["employer"].as_str().unwrap_or("");
            let position=info["position"].as_str().unwrap_or("");
            let (status,hint)=wording(&kind);
            let body=json!({"from":from,"to":[to],"subject":format!("{status}: {position} at {employer}"),"text":format!("{status}\nEmployer: {employer}\nPosition: {position}\nApplication: {}\nTime: {}\n{hint}",info["url"].as_str().unwrap_or(""),info["time"].as_str().unwrap_or(""))});
            let response=client.post(format!("{base}/emails")).bearer_auth(&key).header("Idempotency-Key",id.to_string()).json(&body).send().await?.error_for_status()?.json::<Value>().await?;
            let receipt=response["id"].as_str().ok_or_else(||anyhow::anyhow!("Email provider returned no message identifier"))?;
            // Acceptance is not delivery. A later status read must confirm delivered.
            Ok(("accepted".into(),Some(receipt.into())))
        }.await;
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

/// Subject status and next step per notification kind. Anything that needs the owner says what to do.
fn wording(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "submitted" => ("Confirmed submitted", "The employer confirmed receipt."),
        "submission_uncertain" => ("Check with the employer", "The form may have been sent, but no confirmation was seen. It will not be retried; check with the employer before applying again."),
        "failed" => ("Application failed", "Nothing was confirmed as sent. Open Opportunities to retry it."),
        "manual_required" => ("Complete on the employer website", "Automation cannot finish this form. Open Opportunities for the employer link."),
        "source_failing" => ("Source not refreshing", "Its listings are kept, not closed, while it fails. Check whether the employer moved its career site."),
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
        ]
        .iter()
        .map(|k| super::wording(k).0)
        .collect();
        assert_eq!(subjects.len(), 6);
    }
}
