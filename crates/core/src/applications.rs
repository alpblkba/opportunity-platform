//! Application tracking, facts, documents, authorization and notifications.
//!
//! The state machine is the point of this module. An application moves between states only through the functions here, every move writes an event, and a submission writes an attempt whose idempotency key is what makes applying twice impossible rather than merely unlikely.

use chrono::{DateTime, Duration, Utc};
use opportunity_domain::authorization::{self, Authorization, Policy, SubmissionRequest, Token};
use opportunity_domain::{content_hash, Fact, Route};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

use crate::db::Db;
use crate::profiles::{self, ProfileError};

pub type AppError = ProfileError;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Application {
    pub id: Uuid,
    pub opportunity_id: Uuid,
    pub profile_id: Option<Uuid>,
    pub status: String,
    pub route: Option<String>,
    pub route_reason: Option<String>,
    pub cv_document_id: Option<Uuid>,
    pub dossier_body: Option<String>,
    pub dossier_facts: serde_json::Value,
    pub notes: Option<String>,
    pub follow_up_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct StoredFact {
    pub id: Uuid,
    pub key: String,
    pub value: String,
    pub category: String,
    pub sensitive: bool,
    pub approved: bool,
    pub provenance: String,
    pub version: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewFact {
    pub key: String,
    pub value: String,
    #[serde(default = "other_category")]
    pub category: String,
    #[serde(default)]
    pub sensitive: bool,
    #[serde(default)]
    pub provenance: Option<String>,
}

fn other_category() -> String {
    "other".into()
}

/// Record a fact. A new value for an existing key supersedes the old one rather than overwriting it, so the history of what was claimed remains inspectable, and the new version starts unapproved because the owner has not seen it yet.
pub async fn upsert_fact(pool: &Db, fact: NewFact) -> Result<StoredFact, AppError> {
    if fact.key.trim().is_empty() || fact.value.trim().is_empty() {
        return Err(AppError::Invalid(
            "a fact needs both a key and a value".into(),
        ));
    }
    let owner_id = profiles::local_owner(pool).await?;
    let mut tx = pool.begin().await?;
    let existing: Option<(Uuid, i32)> = sqlx::query_as(
        "select id, version from applicant_facts where owner_id = $1 and key = $2 and superseded_by is null",
    )
    .bind(owner_id)
    .bind(fact.key.trim())
    .fetch_optional(&mut *tx)
    .await?;

    let version = existing.map(|(_, v)| v + 1).unwrap_or(1);
    // The id is chosen here so the previous version can be pointed at the new one before it exists. Inserting first would briefly leave two current rows for one key, which the partial unique index correctly refuses.
    let id = Uuid::new_v4();
    if let Some((previous, _)) = existing {
        sqlx::query(
            "update applicant_facts set superseded_by = $2, updated_at = now() where id = $1",
        )
        .bind(previous)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }

    let row = sqlx::query_as::<_, StoredFact>(
        "insert into applicant_facts (id, owner_id, key, value, category, sensitive, provenance, version)
         values ($1,$2,$3,$4,$5,$6,$7,$8)
         returning id, key, value, category, sensitive, approved, provenance, version, created_at",
    )
    .bind(id)
    .bind(owner_id)
    .bind(fact.key.trim())
    .bind(fact.value.trim())
    .bind(&fact.category)
    .bind(fact.sensitive)
    .bind(fact.provenance.as_deref().unwrap_or("manual"))
    .bind(version)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(row)
}

/// Approve a fact. Only an approved fact is ever written into a form, which is the mechanism that keeps the system from inventing a qualification.
pub async fn approve_fact(pool: &Db, id: Uuid, approved: bool) -> Result<StoredFact, AppError> {
    sqlx::query_as::<_, StoredFact>(
        "update applicant_facts set approved = $2, approved_at = case when $2 then now() else null end, updated_at = now()
         where id = $1
         returning id, key, value, category, sensitive, approved, provenance, version, created_at",
    )
    .bind(id)
    .bind(approved)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound(id))
}

pub async fn list_facts(pool: &Db, only_current: bool) -> Result<Vec<StoredFact>, AppError> {
    let sql = if only_current {
        "select id, key, value, category, sensitive, approved, provenance, version, created_at
         from applicant_facts where superseded_by is null order by category, key"
    } else {
        "select id, key, value, category, sensitive, approved, provenance, version, created_at
         from applicant_facts order by key, version desc"
    };
    Ok(sqlx::query_as::<_, StoredFact>(sql).fetch_all(pool).await?)
}

/// The approved facts the form mapper is allowed to use.
pub async fn approved_facts(pool: &Db) -> Result<Vec<Fact>, AppError> {
    let rows = sqlx::query_as::<_, (String, String, bool)>(
        "select key, value, sensitive from applicant_facts where approved and superseded_by is null",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(key, value, sensitive)| Fact {
            key,
            value,
            sensitive,
        })
        .collect())
}

/// The approved reusable answers, as the form mapper consumes them.
pub async fn approved_answers(pool: &Db) -> Result<Vec<(String, String, bool)>, AppError> {
    Ok(sqlx::query_as::<_, (String, String, bool)>(
        "select question_key, answer, approved from reusable_answers",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn create_application(
    pool: &Db,
    opportunity_id: Uuid,
    profile_id: Option<Uuid>,
) -> Result<Application, AppError> {
    let owner_id = profiles::local_owner(pool).await?;
    // The uniqueness constraint means a second create returns the existing application rather than a second one.
    let row = sqlx::query_as::<_, Application>(
        "insert into applications (owner_id, opportunity_id, profile_id)
         values ($1,$2,$3)
         on conflict (owner_id, opportunity_id) do update set updated_at = now()
         returning id, opportunity_id, profile_id, status, route, route_reason, cv_document_id,
                   dossier_body, dossier_facts, notes, follow_up_at, created_at, updated_at",
    )
    .bind(owner_id)
    .bind(opportunity_id)
    .bind(profile_id)
    .fetch_one(pool)
    .await?;
    // Both timestamps come from the same transaction clock on a real insert; a conflict only moves updated_at. Recording "created" again would put a second birth in a history whose job is to be trustworthy.
    if row.created_at == row.updated_at {
        record_event(pool, row.id, "created", serde_json::json!({})).await?;
    }
    Ok(row)
}

pub async fn get_application(pool: &Db, id: Uuid) -> Result<Application, AppError> {
    sqlx::query_as::<_, Application>(
        "select id, opportunity_id, profile_id, status, route, route_reason, cv_document_id,
                dossier_body, dossier_facts, notes, follow_up_at, created_at, updated_at
         from applications where id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound(id))
}

pub async fn list_applications(pool: &Db) -> Result<Vec<Application>, AppError> {
    Ok(sqlx::query_as::<_, Application>(
        "select id, opportunity_id, profile_id, status, route, route_reason, cv_document_id,
                dossier_body, dossier_facts, notes, follow_up_at, created_at, updated_at
         from applications order by updated_at desc",
    )
    .fetch_all(pool)
    .await?)
}

pub async fn set_status(
    pool: &Db,
    id: Uuid,
    status: &str,
    detail: serde_json::Value,
) -> Result<Application, AppError> {
    let row = sqlx::query_as::<_, Application>(
        "update applications set status = $2, updated_at = now() where id = $1
         returning id, opportunity_id, profile_id, status, route, route_reason, cv_document_id,
                   dossier_body, dossier_facts, notes, follow_up_at, created_at, updated_at",
    )
    .bind(id)
    .bind(status)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound(id))?;
    record_event(pool, id, &format!("status:{status}"), detail).await?;
    Ok(row)
}

pub async fn set_route(pool: &Db, id: Uuid, route: Route, reason: &str) -> Result<(), AppError> {
    sqlx::query(
        "update applications set route = $2, route_reason = $3, updated_at = now() where id = $1",
    )
    .bind(id)
    .bind(route.as_str())
    .bind(reason)
    .execute(pool)
    .await?;
    record_event(
        pool,
        id,
        "routed",
        serde_json::json!({ "route": route.as_str(), "reason": reason }),
    )
    .await?;
    Ok(())
}

pub async fn record_event(
    pool: &Db,
    application_id: Uuid,
    kind: &str,
    detail: serde_json::Value,
) -> Result<(), AppError> {
    sqlx::query("insert into application_events (application_id, kind, detail) values ($1,$2,$3)")
        .bind(application_id)
        .bind(kind)
        .bind(detail)
        .execute(pool)
        .await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Event {
    pub id: Uuid,
    pub kind: String,
    pub detail: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

pub async fn list_events(pool: &Db, application_id: Uuid) -> Result<Vec<Event>, AppError> {
    Ok(sqlx::query_as::<_, Event>(
        "select id, kind, detail, created_at from application_events where application_id = $1 order by created_at",
    )
    .bind(application_id)
    .fetch_all(pool)
    .await?)
}

// ---------------------------------------------------------------------------
// Review tasks
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ReviewTask {
    pub id: Uuid,
    pub application_id: Uuid,
    pub field_name: String,
    pub question_text: String,
    pub field_type: String,
    pub required: bool,
    pub sensitive: bool,
    pub draft_answer: Option<String>,
    pub answer: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

/// Replace the open questions for an application. Answers already given are kept, so re-preparing does not throw away the owner's work.
pub async fn upsert_review_tasks(
    pool: &Db,
    application_id: Uuid,
    questions: &[opportunity_domain::OpenQuestion],
) -> Result<usize, AppError> {
    let mut tx = pool.begin().await?;
    for question in questions {
        sqlx::query(
            "insert into review_tasks (application_id, field_name, question_text, field_type, required, sensitive, draft_answer)
             values ($1,$2,$3,$4,$5,$6,$7)
             on conflict (application_id, field_name) do update set
                question_text = excluded.question_text,
                required = excluded.required,
                sensitive = excluded.sensitive,
                draft_answer = excluded.draft_answer",
        )
        .bind(application_id)
        .bind(&question.field)
        .bind(&question.label)
        .bind(format!("{:?}", question.kind).to_lowercase())
        .bind(question.required)
        .bind(question.sensitive)
        .bind(question.draft.as_deref())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(questions.len())
}

pub async fn list_review_tasks(
    pool: &Db,
    application_id: Option<Uuid>,
) -> Result<Vec<ReviewTask>, AppError> {
    Ok(sqlx::query_as::<_, ReviewTask>(
        "select id, application_id, field_name, question_text, field_type, required, sensitive,
                draft_answer, answer, status, created_at, resolved_at
         from review_tasks
         where ($1::uuid is null or application_id = $1) and status = 'open'
         order by created_at",
    )
    .bind(application_id)
    .fetch_all(pool)
    .await?)
}

/// Record the owner's answer to one open question. This is the hook an external assistant uses: list, inspect, answer, resolve, all through the same state machine.
pub async fn answer_review_task(pool: &Db, id: Uuid, answer: &str) -> Result<ReviewTask, AppError> {
    if answer.trim().is_empty() {
        return Err(AppError::Invalid("an answer cannot be empty".into()));
    }
    let task = sqlx::query_as::<_, ReviewTask>(
        "update review_tasks set answer = $2, status = 'answered', resolved_at = now()
         where id = $1
         returning id, application_id, field_name, question_text, field_type, required, sensitive,
                   draft_answer, answer, status, created_at, resolved_at",
    )
    .bind(id)
    .bind(answer.trim())
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound(id))?;
    record_event(
        pool,
        task.application_id,
        "review_answered",
        serde_json::json!({ "field": task.field_name }),
    )
    .await?;
    Ok(task)
}

/// Answers the owner has given for one application, ready to be merged into the fill payload.
pub async fn answered_tasks(
    pool: &Db,
    application_id: Uuid,
) -> Result<Vec<(String, String)>, AppError> {
    Ok(sqlx::query_as::<_, (String, String)>(
        "select field_name, answer from review_tasks
         where application_id = $1 and status = 'answered' and answer is not null",
    )
    .bind(application_id)
    .fetch_all(pool)
    .await?)
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Notification {
    pub id: Uuid,
    pub application_id: Option<Uuid>,
    pub kind: String,
    pub dedupe_key: String,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub delivered_at: Option<DateTime<Utc>>,
}

/// Emit a durable notification. The dedupe key makes "exactly once per transition" a constraint rather than a hope: a repeated call for the same transition updates nothing and creates nothing.
pub async fn notify(
    pool: &Db,
    application_id: Option<Uuid>,
    kind: &str,
    dedupe_key: &str,
    payload: serde_json::Value,
) -> Result<bool, AppError> {
    let owner_id = profiles::local_owner(pool).await?;
    let inserted = sqlx::query(
        "insert into notifications (owner_id, application_id, kind, dedupe_key, payload)
         values ($1,$2,$3,$4,$5)
         on conflict (owner_id, dedupe_key) do nothing",
    )
    .bind(owner_id)
    .bind(application_id)
    .bind(kind)
    .bind(dedupe_key)
    .bind(payload)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(inserted == 1)
}

pub async fn list_notifications(
    pool: &Db,
    undelivered_only: bool,
) -> Result<Vec<Notification>, AppError> {
    let sql = if undelivered_only {
        "select id, application_id, kind, dedupe_key, payload, created_at, delivered_at
         from notifications where delivered_at is null order by created_at desc limit 100"
    } else {
        "select id, application_id, kind, dedupe_key, payload, created_at, delivered_at
         from notifications order by created_at desc limit 100"
    };
    Ok(sqlx::query_as::<_, Notification>(sql)
        .fetch_all(pool)
        .await?)
}

pub async fn mark_delivered(pool: &Db, id: Uuid) -> Result<(), AppError> {
    sqlx::query(
        "update notifications set delivered_at = now() where id = $1 and delivered_at is null",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Authorization
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct StoredPolicy {
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
    pub source_keys: Vec<String>,
    pub require_all_fields_mapped: bool,
    pub forbid_sensitive_questions: bool,
    pub forbid_free_text: bool,
    pub max_per_day: i32,
    pub max_total: Option<i32>,
    pub submissions_made: i32,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewPolicy {
    pub name: String,
    #[serde(default)]
    pub source_keys: Vec<String>,
    #[serde(default)]
    pub max_per_day: Option<i32>,
    #[serde(default)]
    pub max_total: Option<i32>,
    #[serde(default)]
    pub expires_in_days: Option<i64>,
}

const POLICY_COLUMNS: &str = "id, name, enabled, source_keys, require_all_fields_mapped,
    forbid_sensitive_questions, forbid_free_text, max_per_day, max_total, submissions_made,
    expires_at, revoked_at, created_at";

/// Create a standing authorization. It is created disabled: the owner enables it as a separate, deliberate act.
pub async fn create_policy(pool: &Db, policy: NewPolicy) -> Result<StoredPolicy, AppError> {
    if policy.name.trim().is_empty() {
        return Err(AppError::Invalid("a policy needs a name".into()));
    }
    if policy.source_keys.is_empty() {
        return Err(AppError::Invalid(
            "a policy must name the sources it covers; one that names none would cover everything by accident".into(),
        ));
    }
    let owner_id = profiles::local_owner(pool).await?;
    let expires_at = policy
        .expires_in_days
        .map(|days| Utc::now() + Duration::days(days));
    Ok(sqlx::query_as::<_, StoredPolicy>(&format!(
        "insert into authorization_policies (owner_id, name, source_keys, max_per_day, max_total, expires_at)
         values ($1,$2,$3,$4,$5,$6) returning {POLICY_COLUMNS}"
    ))
    .bind(owner_id)
    .bind(policy.name.trim())
    .bind(&policy.source_keys)
    .bind(policy.max_per_day.unwrap_or(5))
    .bind(policy.max_total)
    .bind(expires_at)
    .fetch_one(pool)
    .await?)
}

pub async fn list_policies(pool: &Db) -> Result<Vec<StoredPolicy>, AppError> {
    Ok(sqlx::query_as::<_, StoredPolicy>(&format!(
        "select {POLICY_COLUMNS} from authorization_policies order by name"
    ))
    .fetch_all(pool)
    .await?)
}

/// Enable, pause or revoke a policy. Revoking is permanent; pausing is simply disabling.
pub async fn set_policy_state(
    pool: &Db,
    id: Uuid,
    enabled: bool,
    revoke: bool,
) -> Result<StoredPolicy, AppError> {
    sqlx::query_as::<_, StoredPolicy>(&format!(
        "update authorization_policies set
            enabled = case when $3 then false else $2 end,
            revoked_at = case when $3 then now() else revoked_at end,
            updated_at = now()
         where id = $1 returning {POLICY_COLUMNS}"
    ))
    .bind(id)
    .bind(enabled)
    .bind(revoke)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound(id))
}

/// Issue a one-use individual authorization bound to this application, destination and payload.
pub async fn issue_token(
    pool: &Db,
    application_id: Uuid,
    destination: &str,
    source_key: &str,
    payload_hash: &str,
    valid_for_minutes: i64,
) -> Result<Uuid, AppError> {
    let owner_id = profiles::local_owner(pool).await?;
    let id: Uuid = sqlx::query_scalar(
        "insert into authorization_tokens (owner_id, application_id, payload_hash, destination, source_key, expires_at)
         values ($1,$2,$3,$4,$5,$6) returning id",
    )
    .bind(owner_id)
    .bind(application_id)
    .bind(payload_hash)
    .bind(destination)
    .bind(source_key)
    .bind(Utc::now() + Duration::minutes(valid_for_minutes.clamp(1, 1440)))
    .fetch_one(pool)
    .await?;
    record_event(
        pool,
        application_id,
        "authorization_issued",
        serde_json::json!({ "token": id }),
    )
    .await?;
    Ok(id)
}

#[derive(Debug, Clone, FromRow)]
struct TokenRow {
    id: Uuid,
    payload_hash: String,
    destination: String,
    expires_at: DateTime<Utc>,
    used_at: Option<DateTime<Utc>>,
}

/// Decide whether a submission may proceed, reading every input from the database so the verdict reflects the state at this moment rather than when the page was loaded.
#[derive(Debug, Clone)]
pub struct AuthorizationContext<'a> {
    pub application_id: Uuid,
    pub source_key: &'a str,
    pub destination: &'a str,
    pub payload_hash: &'a str,
    pub route: Route,
    pub open_questions: usize,
    pub has_sensitive_question: bool,
    pub has_free_text: bool,
}

pub async fn decide_authorization(
    pool: &Db,
    context: &AuthorizationContext<'_>,
) -> Result<(Authorization, Option<Uuid>, Option<Uuid>), AppError> {
    let AuthorizationContext {
        application_id,
        source_key,
        destination,
        payload_hash,
        route,
        open_questions,
        has_sensitive_question,
        has_free_text,
    } = *context;
    // Any attempt that is not an outright failure blocks a further one. An uncertain outcome counts, which is what stops a blind retry.
    let already_attempted: bool = sqlx::query_scalar(
        "select exists (select 1 from application_attempts where application_id = $1 and state in ('submitted','unknown'))",
    )
    .bind(application_id)
    .fetch_one(pool)
    .await?;

    let token_row = sqlx::query_as::<_, TokenRow>(
        "select id, payload_hash, destination, expires_at, used_at
         from authorization_tokens where application_id = $1 and used_at is null
         order by created_at desc limit 1",
    )
    .bind(application_id)
    .fetch_optional(pool)
    .await?;

    let policy_rows = list_policies(pool).await?;
    // Counted per policy, so one policy's activity cannot exhaust another's daily allowance.
    let today: Vec<(Uuid, i64)> = sqlx::query_as(
        "select policy_id, count(*) from application_attempts
         where authorization_kind = 'policy' and policy_id is not null
           and state in ('submitted','unknown') and started_at >= now() - interval '1 day'
         group by policy_id",
    )
    .fetch_all(pool)
    .await?;

    let policies: Vec<Policy> = policy_rows
        .iter()
        .map(|p| Policy {
            name: p.name.clone(),
            enabled: p.enabled,
            source_keys: p.source_keys.clone(),
            require_all_fields_mapped: p.require_all_fields_mapped,
            forbid_sensitive_questions: p.forbid_sensitive_questions,
            forbid_free_text: p.forbid_free_text,
            max_per_day: p.max_per_day,
            max_total: p.max_total,
            submissions_made: p.submissions_made,
            expires_at: p.expires_at,
            revoked_at: p.revoked_at,
            submissions_today: today
                .iter()
                .find(|(id, _)| *id == p.id)
                .map(|(_, count)| *count as i32)
                .unwrap_or(0),
        })
        .collect();

    let token = token_row.as_ref().map(|t| Token {
        payload_hash: t.payload_hash.clone(),
        destination: t.destination.clone(),
        expires_at: t.expires_at,
        used_at: t.used_at,
    });

    let request = SubmissionRequest {
        source_key: source_key.to_owned(),
        destination: destination.to_owned(),
        payload_hash: payload_hash.to_owned(),
        route,
        open_questions,
        has_sensitive_question,
        has_free_text,
        already_attempted,
    };

    let decision = authorization::authorize(&request, token.as_ref(), &policies, Utc::now());
    let policy_id = match &decision {
        Authorization::Policy { policy, .. } => {
            policy_rows.iter().find(|p| &p.name == policy).map(|p| p.id)
        }
        _ => None,
    };
    Ok((decision, policy_id, token_row.map(|t| t.id)))
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Attempt {
    pub id: Uuid,
    pub idempotency_key: String,
    pub state: String,
    pub destination: String,
    pub payload_hash: String,
    pub authorization_kind: Option<String>,
    pub receipt: Option<String>,
    pub diagnostic: Option<String>,
    pub evidence: serde_json::Value,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// The key that makes submission idempotent. Two requests for the same application, destination and payload produce the same key, and the unique index turns the second into a no-op.
/// How many times the same answers may be sent to the same form after failures. A form that refuses the same payload twenty times is not going to accept the twenty-first, and the owner should look at it rather than the machine keep trying.
const MAX_ATTEMPTS_PER_PAYLOAD: u32 = 20;

pub fn idempotency_key(application_id: Uuid, destination: &str, payload_hash: &str) -> String {
    attempt_key(application_id, destination, payload_hash, 0)
}

/// The key for the nth attempt at one payload. Attempt 0 is the original; later numbers exist only because an earlier attempt was verified to have sent nothing.
fn attempt_key(
    application_id: Uuid,
    destination: &str,
    payload_hash: &str,
    attempt_number: u32,
) -> String {
    let scope = if attempt_number == 0 {
        "submit".to_owned()
    } else {
        format!("submit#{attempt_number}")
    };
    content_hash(
        &application_id.to_string(),
        Some(destination),
        Some(payload_hash),
        &scope,
    )
}

/// Claim the right to submit.
///
/// An attempt that already exists for this exact payload is returned rather than repeated, which is what stops a retried request applying twice. The exception is an attempt that failed: a failure means the form refused or the browser died before anything was sent, which the worker establishes rather than assumes, so the owner may try the same answers again. `submitted` and `unknown` are both final here; `unknown` especially, because nobody knows whether it arrived.
pub async fn begin_attempt(
    pool: &Db,
    application_id: Uuid,
    destination: &str,
    payload_hash: &str,
    authorization_kind: &str,
    policy_id: Option<Uuid>,
    token_id: Option<Uuid>,
) -> Result<(Attempt, bool), AppError> {
    let mut tx: Transaction<'_, Postgres> = pool.begin().await?;
    let mut key = idempotency_key(application_id, destination, payload_hash);
    for attempt_number in 0..MAX_ATTEMPTS_PER_PAYLOAD {
        key = attempt_key(application_id, destination, payload_hash, attempt_number);
        let existing = sqlx::query_as::<_, Attempt>(
            "select id, idempotency_key, state, destination, payload_hash, authorization_kind, receipt,
                    diagnostic, evidence, started_at, finished_at
             from application_attempts where idempotency_key = $1",
        )
        .bind(&key)
        .fetch_optional(&mut *tx)
        .await?;
        match existing {
            // Nothing was sent, so this is a fresh attempt rather than a repeat of that one. The failed row stays for the history.
            Some(attempt) if attempt.state == "failed" => continue,
            Some(attempt) => {
                tx.commit().await?;
                return Ok((attempt, false));
            }
            None => break,
        }
    }

    let attempt = sqlx::query_as::<_, Attempt>(
        "insert into application_attempts (application_id, idempotency_key, destination, payload_hash, authorization_kind, policy_id, token_id)
         values ($1,$2,$3,$4,$5,$6,$7)
         returning id, idempotency_key, state, destination, payload_hash, authorization_kind, receipt,
                   diagnostic, evidence, started_at, finished_at",
    )
    .bind(application_id)
    .bind(&key)
    .bind(destination)
    .bind(payload_hash)
    .bind(authorization_kind)
    .bind(policy_id)
    .bind(token_id)
    .fetch_one(&mut *tx)
    .await?;

    // Spending the token and counting the policy happen in the same transaction that claims the attempt, so an authorization cannot be used twice even under a race.
    if let Some(token) = token_id {
        sqlx::query("update authorization_tokens set used_at = now(), used_by_attempt = $2 where id = $1 and used_at is null")
            .bind(token)
            .bind(attempt.id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(policy) = policy_id {
        sqlx::query("update authorization_policies set submissions_made = submissions_made + 1, updated_at = now() where id = $1")
            .bind(policy)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok((attempt, true))
}

/// Close an attempt with what actually happened.
pub async fn finish_attempt(
    pool: &Db,
    attempt_id: Uuid,
    state: &str,
    receipt: Option<&str>,
    diagnostic: Option<&str>,
    evidence: serde_json::Value,
) -> Result<(), AppError> {
    sqlx::query(
        "update application_attempts set state = $2, receipt = $3, diagnostic = $4, evidence = $5, finished_at = now()
         where id = $1",
    )
    .bind(attempt_id)
    .bind(state)
    .bind(receipt)
    .bind(diagnostic)
    .bind(evidence)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_attempts(pool: &Db, application_id: Uuid) -> Result<Vec<Attempt>, AppError> {
    Ok(sqlx::query_as::<_, Attempt>(
        "select id, idempotency_key, state, destination, payload_hash, authorization_kind, receipt,
                diagnostic, evidence, started_at, finished_at
         from application_attempts where application_id = $1 order by started_at desc",
    )
    .bind(application_id)
    .fetch_all(pool)
    .await?)
}
