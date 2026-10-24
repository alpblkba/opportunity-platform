use chrono::{DateTime, Utc};
use opportunity_domain::{SourceEndpoint, SourceHealth, SourceKind, SourceSupport};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::db::Db;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Source {
    pub id: Uuid,
    pub key: String,
    pub kind: String,
    pub display_name: String,
    pub organization_id: Option<Uuid>,
    pub endpoint: serde_json::Value,
    pub enabled: bool,
    pub refresh_hours: i32,
    pub tags: Vec<String>,
    pub health: String,
    pub support: String,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_outcome: Option<String>,
    pub last_diagnostic: Option<String>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub consecutive_failures: i32,
    pub backoff_until: Option<DateTime<Utc>>,
}

impl Source {
    pub fn typed_endpoint(&self) -> Result<SourceEndpoint, serde_json::Error> {
        serde_json::from_value(self.endpoint.clone())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewSource {
    pub key: String,
    pub display_name: String,
    pub organization: String,
    pub endpoint: SourceEndpoint,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_refresh_hours")]
    pub refresh_hours: i32,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_refresh_hours() -> i32 {
    6
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SourceUpdate {
    pub display_name: Option<String>,
    pub enabled: Option<bool>,
    pub refresh_hours: Option<i32>,
    pub tags: Option<Vec<String>>,
    pub endpoint: Option<SourceEndpoint>,
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("source configuration is invalid: {0}")]
    Invalid(String),
    #[error("no source with id {0}")]
    NotFound(Uuid),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

pub async fn list(pool: &Db) -> Result<Vec<Source>, SourceError> {
    let rows = sqlx::query_as::<_, Source>(
        "select id, key, kind, display_name, organization_id, endpoint, enabled, refresh_hours, tags, health, support, last_attempt_at, last_success_at, last_outcome, last_diagnostic, next_run_at, consecutive_failures, backoff_until, next_run_at, consecutive_failures, backoff_until
         from sources order by key",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get(pool: &Db, id: Uuid) -> Result<Source, SourceError> {
    sqlx::query_as::<_, Source>(
        "select id, key, kind, display_name, organization_id, endpoint, enabled, refresh_hours, tags, health, support, last_attempt_at, last_success_at, last_outcome, last_diagnostic, next_run_at, consecutive_failures, backoff_until, next_run_at, consecutive_failures, backoff_until
         from sources where id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(SourceError::NotFound(id))
}

pub async fn get_by_key(pool: &Db, key: &str) -> Result<Option<Source>, SourceError> {
    let row = sqlx::query_as::<_, Source>(
        "select id, key, kind, display_name, organization_id, endpoint, enabled, refresh_hours, tags, health, support, last_attempt_at, last_success_at, last_outcome, last_diagnostic, next_run_at, consecutive_failures, backoff_until, next_run_at, consecutive_failures, backoff_until
         from sources where key = $1",
    )
    .bind(key)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Find or create the organization row for a display name. Identity is the normalized name, so two spellings of one employer converge on one row and its postings can be grouped.
pub async fn upsert_organization(pool: &Db, display_name: &str) -> Result<Uuid, SourceError> {
    let normalized = opportunity_domain::normalize_organization(display_name);
    if normalized.is_empty() {
        return Err(SourceError::Invalid("organization name is empty".into()));
    }
    let id: Uuid = sqlx::query_scalar(
        "insert into organizations (normalized_name, display_name) values ($1, $2)
         on conflict (normalized_name) do update set updated_at = now()
         returning id",
    )
    .bind(&normalized)
    .bind(display_name.trim())
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Register a source. The endpoint is validated at this boundary so an unconfigured template row can never reach the scanner and be reported as a checked source.
pub async fn create(pool: &Db, new_source: NewSource) -> Result<Source, SourceError> {
    new_source
        .endpoint
        .validate()
        .map_err(SourceError::Invalid)?;
    if new_source.key.trim().is_empty() {
        return Err(SourceError::Invalid("key is empty".into()));
    }
    if !(1..=720).contains(&new_source.refresh_hours) {
        return Err(SourceError::Invalid(
            "refresh_hours must be between 1 and 720".into(),
        ));
    }
    let organization_id = upsert_organization(pool, &new_source.organization).await?;
    let kind = new_source.endpoint.kind();
    // The support label states what is actually known about the kind at registration: no adapter means a human link, an adapter that cannot yet be run live means fixture only, and anything else is simply unchecked.
    let support = if !kind.has_adapter() {
        SourceSupport::ManualLink
    } else if kind.is_fixture_only_until_opt_in() {
        SourceSupport::FixtureOnly
    } else {
        SourceSupport::NotConfigured
    };
    let row = sqlx::query_as::<_, Source>(
        "insert into sources (key, kind, display_name, organization_id, endpoint, enabled, refresh_hours, tags, support, next_run_at)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, case when $6 then now() else null end)
         returning id, key, kind, display_name, organization_id, endpoint, enabled, refresh_hours, tags, health, support, last_attempt_at, last_success_at, last_outcome, last_diagnostic, next_run_at, consecutive_failures, backoff_until",
    )
    .bind(new_source.key.trim())
    .bind(kind.as_str())
    .bind(new_source.display_name.trim())
    .bind(organization_id)
    .bind(serde_json::to_value(&new_source.endpoint)?)
    .bind(new_source.enabled)
    .bind(new_source.refresh_hours)
    .bind(&new_source.tags)
    .bind(support.as_str())
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn update(pool: &Db, id: Uuid, patch: SourceUpdate) -> Result<Source, SourceError> {
    let existing = get(pool, id).await?;
    if let Some(endpoint) = &patch.endpoint {
        endpoint.validate().map_err(SourceError::Invalid)?;
        if endpoint.kind().as_str() != existing.kind {
            return Err(SourceError::Invalid(
                "endpoint kind cannot change; register a new source instead".into(),
            ));
        }
    }
    if let Some(hours) = patch.refresh_hours {
        if !(1..=720).contains(&hours) {
            return Err(SourceError::Invalid(
                "refresh_hours must be between 1 and 720".into(),
            ));
        }
    }
    let endpoint = match patch.endpoint {
        Some(endpoint) => serde_json::to_value(endpoint)?,
        None => existing.endpoint.clone(),
    };
    let row = sqlx::query_as::<_, Source>(
        "update sources set
            display_name = coalesce($2, display_name),
            enabled = coalesce($3, enabled),
            refresh_hours = coalesce($4, refresh_hours),
            tags = coalesce($5, tags),
            endpoint = $6,
            -- Enabling a source makes it due immediately; disabling clears its schedule so it is not picked up while off.
            next_run_at = case
                when coalesce($3, enabled) then coalesce(next_run_at, now())
                else null
            end,
            updated_at = now()
         where id = $1
         returning id, key, kind, display_name, organization_id, endpoint, enabled, refresh_hours, tags, health, support, last_attempt_at, last_success_at, last_outcome, last_diagnostic, next_run_at, consecutive_failures, backoff_until",
    )
    .bind(id)
    .bind(patch.display_name)
    .bind(patch.enabled)
    .bind(patch.refresh_hours)
    .bind(patch.tags)
    .bind(endpoint)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn delete(pool: &Db, id: Uuid) -> Result<(), SourceError> {
    let affected = sqlx::query("delete from sources where id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(SourceError::NotFound(id));
    }
    Ok(())
}

/// Record the result of one attempt against a source. Support only advances to `verified_live` on a sweep that actually read the source; it never advances because an adapter exists.
#[allow(clippy::too_many_arguments)]
pub async fn record_attempt(
    pool: &Db,
    id: Uuid,
    outcome: opportunity_domain::RunOutcome,
    diagnostic: Option<&str>,
) -> Result<(), SourceError> {
    let health = SourceHealth::from_outcome(outcome);
    let success = outcome.is_healthy();
    let support = match outcome {
        o if o.is_complete_sweep() => Some(SourceSupport::VerifiedLive),
        opportunity_domain::RunOutcome::AccessDenied => Some(SourceSupport::Blocked),
        _ => None,
    };
    sqlx::query(
        "update sources set
            last_attempt_at = now(),
            last_success_at = case when $2 then now() else last_success_at end,
            last_outcome = $3,
            last_diagnostic = $4,
            health = case when support in ('manual_link', 'fixture_only') and $3 <> 'ok_complete' then health else $5 end,
            -- A complete sweep always earns verified_live. Otherwise a label that describes the adapter rather than the employer is kept: a manual entry has no adapter, and a fixture-only kind was refused by our own policy, not by the source, so neither may be downgraded to blocked.
            support = case
                when $6 = 'verified_live' then $6
                when support in ('manual_link', 'fixture_only') then support
                else coalesce($6, support)
            end,
            updated_at = now()
         where id = $1",
    )
    .bind(id)
    .bind(success)
    .bind(outcome.as_str())
    .bind(diagnostic)
    .bind(health.as_str())
    .bind(support.map(|s| s.as_str()))
    .execute(pool)
    .await?;
    Ok(())
}

/// Sources the scheduler should scan now: enabled, backed by an adapter, not in backoff, and either never scheduled or past their next run. Ordered oldest-first so a long backlog drains fairly.
pub async fn due_by_schedule(pool: &Db) -> Result<Vec<Source>, SourceError> {
    let kinds: Vec<String> = [
        SourceKind::Greenhouse,
        SourceKind::Lever,
        SourceKind::Ashby,
        SourceKind::Workday,
        SourceKind::AcademicHtml,
    ]
    .iter()
    .map(|k| k.as_str().to_owned())
    .collect();
    let rows = sqlx::query_as::<_, Source>(
        "select id, key, kind, display_name, organization_id, endpoint, enabled, refresh_hours, tags, health, support, last_attempt_at, last_success_at, last_outcome, last_diagnostic, next_run_at, consecutive_failures, backoff_until
         from sources
         where enabled
           and kind = any($1)
           and (backoff_until is null or backoff_until <= now())
           and (next_run_at is null or next_run_at <= now())
         order by next_run_at nulls first",
    )
    .bind(&kinds)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
