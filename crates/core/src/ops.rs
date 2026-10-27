//! What an operator needs at a glance, read from PostgreSQL and this process. Anything this process
//! cannot measure is reported as unavailable with where to look instead, never as healthy, and the
//! `attention` list names only problems that were actually measured.

use serde_json::{json, Value};

use crate::applications::AppError;
use crate::{worker, Db};

pub async fn status(db: &Db) -> Result<Value, AppError> {
    let queue: Value = sqlx::query_scalar(
        "select jsonb_build_object(
           'queued', count(*) filter (where auto_resume and status in ('saved','ready')),
           'oldest_queued_minutes', (extract(epoch from now() - min(updated_at) filter (where auto_resume and status in ('saved','ready'))) / 60)::int,
           'preparing', count(*) filter (where status = 'preparing'),
           'submitting', count(*) filter (where status = 'submitting'),
           'stuck', count(*) filter (where status in ('preparing','submitting') and updated_at < now() - interval '10 minutes'),
           'uncertain', count(*) filter (where status = 'submission_unknown'))
         from applications",
    )
    .fetch_one(db)
    .await?;
    let sources = source_health(db).await?;
    let notifications: Value = sqlx::query_scalar(
        "select jsonb_build_object(
           'pending', count(*) filter (where delivered_at is null and delivery_state not in ('bounced','failed')),
           'oldest_pending_minutes', (extract(epoch from now() - min(created_at) filter (where delivered_at is null and delivery_state not in ('bounced','failed'))) / 60)::int,
           'undeliverable', count(*) filter (where delivery_state in ('bounced','failed')))
         from notifications",
    )
    .fetch_one(db)
    .await?;

    let count = |v: &Value, key: &str| v[key].as_i64().unwrap_or(0);
    let mut attention = Vec::new();
    if count(&sources, "failing") > 0 {
        attention.push(format!(
            "{} source(s) failing to refresh",
            count(&sources, "failing")
        ));
    }
    if count(&queue, "stuck") > 0 {
        attention.push(format!(
            "{} application(s) stuck preparing or submitting",
            count(&queue, "stuck")
        ));
    }
    if count(&queue, "uncertain") > 0 {
        attention.push(format!(
            "{} submission(s) with an unknown outcome",
            count(&queue, "uncertain")
        ));
    }
    if count(&queue, "oldest_queued_minutes") > 60 {
        attention.push("the application queue has not moved for over an hour".into());
    }
    if count(&notifications, "undeliverable") > 0 {
        attention.push(format!(
            "{} notification(s) could not be delivered",
            count(&notifications, "undeliverable")
        ));
    }
    if crate::mail::configured() && count(&notifications, "oldest_pending_minutes") > 60 {
        attention.push("email notifications have been pending for over an hour".into());
    }

    Ok(json!({
        "release": {"revision": std::env::var("APP_REVISION").ok().filter(|v| !v.is_empty()), "version": env!("CARGO_PKG_VERSION")},
        "database": "ready",
        "queue": queue,
        "sources": sources,
        "notifications": notifications,
        "email_configured": crate::mail::configured(),
        "worker": {"available": worker::worker_available()},
        "memory": memory(),
        "disk": "unavailable here; vm-check.sh measures the VM's disk",
        "backups": "unavailable here; backups are pulled to the owner's machine, and each manifest.json records its time",
        "deployment": "unavailable here; deploy-vm.sh --status reports the running revision and migrations",
        "attention": attention,
    }))
}

/// Source health over every enabled source, for the dashboard and for `status`.
pub async fn source_health(db: &Db) -> Result<Value, AppError> {
    Ok(sqlx::query_scalar(
        "select jsonb_build_object(
           'enabled', count(*) filter (where enabled),
           'failing', count(*) filter (where enabled and consecutive_failures >= 3),
           'never_succeeded', count(*) filter (where enabled and last_success_at is null),
           'stale', count(*) filter (where enabled and last_success_at < now() - make_interval(hours => refresh_hours * 2)),
           'last_success_at', max(last_success_at) filter (where enabled))
         from sources",
    )
    .fetch_one(db)
    .await?)
}

/// From /proc/meminfo where there is one (the Linux container); elsewhere, honestly unavailable.
fn memory() -> Value {
    let Ok(text) = std::fs::read_to_string("/proc/meminfo") else {
        return json!("unavailable on this platform");
    };
    let field = |name: &str| {
        text.lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|kb| kb.parse::<u64>().ok())
            .map(|kb| kb / 1024)
    };
    match (field("MemTotal:"), field("MemAvailable:")) {
        (Some(total), Some(available)) => json!({"total_mb": total, "available_mb": available}),
        _ => json!("unavailable: /proc/meminfo has no MemTotal/MemAvailable"),
    }
}
