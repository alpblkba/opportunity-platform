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
    let rows:Vec<(Uuid,String,Value,Option<String>)>=sqlx::query_as("select n.id,n.kind,jsonb_build_object('employer',org.display_name,'position',o.canonical_title,'url',(select p.url from postings p where p.opportunity_id=o.id order by p.url limit 1),'time',n.created_at,'detail',n.payload),n.delivery_receipt from notifications n join applications a on a.id=n.application_id join opportunities o on o.id=a.opportunity_id join organizations org on org.id=o.organization_id where n.kind in ('submitted','action_required') and n.delivered_at is null and n.delivery_state not in ('bounced','failed') and (n.last_attempt_at is null or n.last_attempt_at<now()-interval '1 minute') order by n.created_at for update of n skip locked limit 5").fetch_all(&mut *tx).await?;
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
            let status=if kind=="submitted"{"Confirmed submitted"}else{"Your answer is needed"};
            let body=json!({"from":from,"to":[to],"subject":format!("{status}: {position} at {employer}"),"text":format!("{status}\nEmployer: {employer}\nPosition: {position}\nApplication: {}\nTime: {}\n{}",info["url"].as_str().unwrap_or(""),info["time"].as_str().unwrap_or(""),if kind=="submitted"{"The employer confirmed receipt."}else{"Open Opportunities to answer the employer's questions."})});
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
