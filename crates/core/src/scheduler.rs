//! Periodic source refresh.
//!
//! All scheduling state is in PostgreSQL rather than in memory, so a restart resumes exactly where the previous process stopped: a source already scanned is not scanned again until it is next due, and a source in backoff stays in backoff. The loop itself is deliberately dumb; the database decides what is due.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use opportunity_adapters::HttpClient;
use opportunity_domain::RunOutcome;
use sqlx::Connection;
use tokio::time::Instant;
use uuid::Uuid;

use crate::db::Db;
use crate::ingest;
use crate::sources::{self, Source, SourceError};

/// How often the loop wakes to ask the database what is due. Sources are refreshed on their own cadence of hours, so a one-minute tick is responsive without being busy.
const TICK: Duration = Duration::from_secs(60);
/// How many one-minute ticks between sweeps of long-dead sessions.
const SESSION_SWEEP_TICKS: u64 = 60;

/// Minimum gap between two requests to the same host, so several sources on one provider stay polite.
const MIN_HOST_INTERVAL: Duration = Duration::from_secs(2);
/// First backoff step after a failure, doubled per consecutive failure.
const BACKOFF_BASE_MINUTES: i64 = 5;
/// Ceiling on backoff, so a source that has been failing for days still gets retried daily.
const BACKOFF_MAX_MINUTES: i64 = 24 * 60;

/// Identifies the one lock that serializes heavy work. An arbitrary constant: all that matters is that every process uses the same number.
const WORK_LOCK_KEY: i64 = 0x0_9F01_5EED_1234_5678_u64 as i64;

/// How often a waiter re-asks for the lock. Short enough that a browser run starts promptly when a sweep ends, long enough not to poll the database for nothing.
const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Held for as long as one piece of heavy work runs.
///
/// Two kinds of work take this lock: a source sweep and a browser run. They are serialized against each other because on a 1 GiB machine they do not fit at the same time: measured, a refresh running underneath an application is enough for the kernel to kill Chromium mid-submission. On a larger machine the lock costs a few seconds of waiting and changes nothing else.
///
/// The connection is detached from the pool rather than borrowed from it. An advisory lock belongs to a session, so a pooled connection handed back while still holding one would carry the lock into whatever used it next and no refresh would ever run again. A detached connection is closed when this value is dropped, and PostgreSQL releases the lock with the session: that is also what makes a killed process recover, since its connection dies with it.
pub struct WorkLease {
    connection: Option<sqlx::PgConnection>,
}

impl WorkLease {
    /// Take the lock, or return `None` because something else is already doing heavy work.
    pub async fn try_acquire(pool: &Db) -> Result<Option<Self>, sqlx::Error> {
        let mut connection = pool.acquire().await?.detach();
        let acquired: bool = sqlx::query_scalar("select pg_try_advisory_lock($1)")
            .bind(WORK_LOCK_KEY)
            .fetch_one(&mut connection)
            .await?;
        if acquired {
            Ok(Some(Self {
                connection: Some(connection),
            }))
        } else {
            let _ = connection.close().await;
            Ok(None)
        }
    }

    /// Wait up to `patience` for the lock, then give up.
    ///
    /// Polling rather than `pg_advisory_lock`, because a blocking lock waits with no ceiling and this is on the path of a request someone is waiting for. Giving up is a better answer than hanging.
    pub async fn acquire_within(
        pool: &Db,
        patience: Duration,
    ) -> Result<Option<Self>, sqlx::Error> {
        let deadline = tokio::time::Instant::now() + patience;
        loop {
            if let Some(lease) = Self::try_acquire(pool).await? {
                return Ok(Some(lease));
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(LOCK_POLL_INTERVAL).await;
        }
    }

    /// Release and close. Dropping without this is safe and releases the lock too, but only once the connection has finished closing; releasing here is what lets the next piece of work start at once.
    pub async fn release(mut self) {
        if let Some(mut connection) = self.connection.take() {
            let _ = sqlx::query("select pg_advisory_unlock($1)")
                .bind(WORK_LOCK_KEY)
                .execute(&mut connection)
                .await;
            let _ = connection.close().await;
        }
    }
}

/// Per-host pacing shared by everything that scans. Keyed by the provider host rather than by source, because several sources can sit behind one provider.
#[derive(Default)]
pub struct HostPacer {
    last_request: HashMap<String, Instant>,
}

impl HostPacer {
    pub async fn wait_for(&mut self, host: &str) {
        if let Some(previous) = self.last_request.get(host) {
            let elapsed = previous.elapsed();
            if elapsed < MIN_HOST_INTERVAL {
                tokio::time::sleep(MIN_HOST_INTERVAL - elapsed).await;
            }
        }
        self.last_request.insert(host.to_owned(), Instant::now());
    }
}

/// Backoff after a failure, doubling per consecutive failure up to a daily ceiling, with jitter so several sources that fail together do not retry in lockstep.
pub fn backoff_minutes(consecutive_failures: i32, jitter_source: Uuid) -> i64 {
    let step = consecutive_failures.clamp(1, 16) - 1;
    let base = BACKOFF_BASE_MINUTES.saturating_mul(1i64 << step.min(12));
    let capped = base.min(BACKOFF_MAX_MINUTES);
    // Deterministic jitter derived from the source id: spreads retries without needing a random number generator or making the schedule untestable.
    let jitter = (jitter_source.as_u128() % 60) as i64 * capped / 600;
    (capped + jitter).min(BACKOFF_MAX_MINUTES * 2)
}

/// Record the outcome of a scheduled scan and set when the source is next due.
pub async fn record_schedule(
    pool: &Db,
    source: &Source,
    outcome: RunOutcome,
) -> Result<DateTime<Utc>, SourceError> {
    let healthy = outcome.is_healthy() || outcome == RunOutcome::Partial;
    let failures = if healthy {
        0
    } else {
        source.consecutive_failures + 1
    };
    let next_in_minutes = if healthy {
        i64::from(source.refresh_hours) * 60
    } else {
        backoff_minutes(failures, source.id)
    };
    let next_run_at = Utc::now() + chrono::Duration::minutes(next_in_minutes);
    sqlx::query(
        "update sources set
            next_run_at = $2,
            consecutive_failures = $3,
            backoff_until = case when $4 then null else $2 end,
            updated_at = now()
         where id = $1",
    )
    .bind(source.id)
    .bind(next_run_at)
    .bind(failures)
    .bind(healthy)
    .execute(pool)
    .await?;
    Ok(next_run_at)
}

/// Sources that are enabled, have an adapter, and whose next run is due. A source that has never been scheduled is due immediately.
pub async fn due_now(pool: &Db) -> Result<Vec<Source>, SourceError> {
    sources::due_by_schedule(pool).await
}

/// Scan every due source once. Returns how many were attempted. Shared by the background loop and the manual `refresh` endpoint so both obey the same pacing and backoff rules.
pub async fn run_due(
    pool: &Db,
    http: &HttpClient,
    pacer: &mut HostPacer,
) -> Result<usize, SourceError> {
    run_due_locked(pool, http, pacer).await
}

async fn run_due_locked(
    pool: &Db,
    http: &HttpClient,
    pacer: &mut HostPacer,
) -> Result<usize, SourceError> {
    let due = due_now(pool).await?;
    if due.is_empty() {
        return Ok(0);
    }
    // One source at a time, awaited in turn. A semaphore used to sit here claiming a concurrency of two that this loop never had, which on a quarter of a shared vCPU would have been the wrong thing to fix: a sweep that takes the machine for itself is worse than one that takes longer.
    let mut attempted = 0usize;
    for source in &due {
        let Ok(endpoint) = source.typed_endpoint() else {
            tracing::warn!(source = %source.key, "skipping a source whose stored endpoint no longer parses");
            continue;
        };
        if !endpoint.kind().has_adapter() {
            continue;
        }
        // Let a pending application run between sources instead of waiting for the entire catalogue.
        let Some(lease) = WorkLease::try_acquire(pool).await? else {
            break;
        };
        pacer.wait_for(&provider_host(&source.kind)).await;
        let scan = opportunity_adapters::scan(http, &endpoint).await;
        let outcome = scan.outcome;
        match ingest::ingest(pool, source, scan).await {
            Ok(summary) => tracing::info!(
                source = %source.key,
                outcome = %summary.outcome,
                records = summary.records_seen,
                "scheduled scan finished"
            ),
            Err(error) => {
                tracing::warn!(source = %source.key, %error, "scheduled scan could not be persisted")
            }
        }
        record_schedule(pool, source, outcome).await?;
        lease.release().await;
        attempted += 1;
    }
    Ok(attempted)
}

/// Host used for pacing. Grouping by provider is what keeps several boards on one provider from being hit at once.
fn provider_host(kind: &str) -> String {
    match kind {
        "greenhouse" => "boards-api.greenhouse.io".into(),
        "lever" => "api.lever.co".into(),
        "ashby" => "api.ashbyhq.com".into(),
        "workday" => "myworkdayjobs.com".into(),
        other => other.to_owned(),
    }
}

/// Start the background refresh loop. It owns no state of its own: everything it needs to resume after a restart is already in the database.
pub fn spawn(pool: Db, http: HttpClient) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut pacer = HostPacer::default();
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut ticks: u64 = 0;
        loop {
            ticker.tick().await;
            ticks += 1;
            match run_due(&pool, &http, &mut pacer).await {
                Ok(0) => {}
                Ok(count) => tracing::info!(count, "scheduled refresh completed"),
                Err(error) => tracing::warn!(%error, "scheduled refresh failed"),
            }
            // Once an hour, on the loop that already exists rather than a timer of its own. Expiry
            // is enforced in the session lookup, so this only stops the table growing for ever.
            if ticks % SESSION_SWEEP_TICKS == 0 {
                match crate::auth::purge_expired_sessions(&pool).await {
                    Ok(0) => {}
                    Ok(removed) => tracing::info!(removed, "removed sessions that ended long ago"),
                    Err(error) => tracing::warn!(%error, "could not sweep old sessions"),
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_with_consecutive_failures_and_is_capped() {
        let id = Uuid::nil();
        let first = backoff_minutes(1, id);
        let second = backoff_minutes(2, id);
        let third = backoff_minutes(3, id);
        assert!(first >= BACKOFF_BASE_MINUTES);
        assert!(second > first, "{second} should exceed {first}");
        assert!(third > second, "{third} should exceed {second}");
        // However long a source has been failing, it is still retried rather than abandoned.
        let forever = backoff_minutes(500, id);
        assert!(
            forever <= BACKOFF_MAX_MINUTES * 2,
            "{forever} exceeded the ceiling"
        );
        assert!(forever >= BACKOFF_MAX_MINUTES);
    }

    #[test]
    fn jitter_spreads_two_sources_that_failed_together() {
        let a = backoff_minutes(4, Uuid::from_u128(7));
        let b = backoff_minutes(4, Uuid::from_u128(59));
        assert_ne!(
            a, b,
            "two sources failing at the same step must not retry in lockstep"
        );
    }

    #[tokio::test]
    async fn the_pacer_spaces_out_requests_to_one_host_but_not_across_hosts() {
        let mut pacer = HostPacer::default();
        pacer.wait_for("api.lever.co").await;
        let other = Instant::now();
        pacer.wait_for("api.ashbyhq.com").await;
        assert!(
            other.elapsed() < MIN_HOST_INTERVAL,
            "a different host must not be delayed"
        );
        let same = Instant::now();
        pacer.wait_for("api.lever.co").await;
        assert!(
            same.elapsed() >= MIN_HOST_INTERVAL / 2,
            "the same host must be paced"
        );
    }

    #[test]
    fn every_adapter_kind_maps_to_its_provider_host_for_pacing() {
        assert_eq!(provider_host("greenhouse"), "boards-api.greenhouse.io");
        assert_eq!(provider_host("lever"), "api.lever.co");
        assert_eq!(provider_host("ashby"), "api.ashbyhq.com");
        assert_eq!(provider_host("workday"), "myworkdayjobs.com");
        // An academic source is paced under its own kind, because each one is a different institution's host.
        assert_eq!(provider_host("academic_html"), "academic_html");
    }
}
