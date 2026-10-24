//! Routes for applicant facts, applications, review tasks, authorization and notifications.
//!
//! These are shaped so an external assistant can drive the review queue through the same state machine the interface uses: list what needs attention, inspect one, record an answer, and resolve it. There is no second path that bypasses the rules.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use opportunity_core::applications::{
    self, Application, Attempt, Event, NewFact, NewPolicy, Notification, ReviewTask, StoredFact,
    StoredPolicy,
};
use opportunity_core::workflow::{self, PrepareResult, SubmitReport};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ApiError;
use crate::routes::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/facts", get(list_facts).post(create_fact))
        .route("/facts/{id}/approve", post(approve_fact))
        .route(
            "/applications",
            get(list_applications).post(create_application),
        )
        .route("/applications/{id}", get(get_application))
        .route("/applications/{id}/events", get(list_events))
        .route("/applications/{id}/attempts", get(list_attempts))
        .route("/applications/{id}/prepare", post(prepare))
        .route(
            "/applications/{id}/authorize-submit",
            post(authorize_submit),
        )
        .route("/applications/{id}/submit", post(submit))
        .route("/review-tasks", get(list_review_tasks))
        .route("/review-tasks/{id}/answer", post(answer_review_task))
        .route("/policies", get(list_policies).post(create_policy))
        .route("/policies/{id}/state", post(set_policy_state))
        .route("/notifications", get(list_notifications))
        .route("/notifications/{id}/delivered", post(mark_delivered))
}

#[derive(Debug, Deserialize)]
struct FactQuery {
    #[serde(default)]
    all_versions: bool,
}

async fn list_facts(
    State(state): State<AppState>,
    Query(params): Query<FactQuery>,
) -> Result<Json<Vec<StoredFact>>, ApiError> {
    Ok(Json(
        applications::list_facts(&state.db, !params.all_versions).await?,
    ))
}

async fn create_fact(
    State(state): State<AppState>,
    Json(fact): Json<NewFact>,
) -> Result<(StatusCode, Json<StoredFact>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(applications::upsert_fact(&state.db, fact).await?),
    ))
}

#[derive(Debug, Deserialize)]
struct ApprovalBody {
    #[serde(default = "yes")]
    approved: bool,
}

fn yes() -> bool {
    true
}

async fn approve_fact(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<ApprovalBody>,
) -> Result<Json<StoredFact>, ApiError> {
    Ok(Json(
        applications::approve_fact(&state.db, id, body.approved).await?,
    ))
}

async fn list_applications(
    State(state): State<AppState>,
) -> Result<Json<Vec<Application>>, ApiError> {
    Ok(Json(applications::list_applications(&state.db).await?))
}

#[derive(Debug, Deserialize)]
struct NewApplication {
    opportunity_id: Uuid,
    #[serde(default)]
    profile_id: Option<Uuid>,
}

async fn create_application(
    State(state): State<AppState>,
    Json(body): Json<NewApplication>,
) -> Result<(StatusCode, Json<Application>), ApiError> {
    let application =
        applications::create_application(&state.db, body.opportunity_id, body.profile_id).await?;
    Ok((StatusCode::CREATED, Json(application)))
}

async fn get_application(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Application>, ApiError> {
    Ok(Json(applications::get_application(&state.db, id).await?))
}

async fn list_events(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<Event>>, ApiError> {
    Ok(Json(applications::list_events(&state.db, id).await?))
}

async fn list_attempts(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<Attempt>>, ApiError> {
    Ok(Json(applications::list_attempts(&state.db, id).await?))
}

#[derive(Debug, Deserialize)]
struct FormBody {
    url: String,
    #[serde(default)]
    source_key: Option<String>,
    #[serde(default)]
    document_path: Option<String>,
}

async fn prepare(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<FormBody>,
) -> Result<Json<PrepareResult>, ApiError> {
    Ok(Json(workflow::prepare(&state.db, id, &body.url).await?))
}

#[derive(Debug, Deserialize)]
struct AuthorizeBody {
    url: String,
    source_key: String,
    #[serde(default = "default_minutes")]
    valid_for_minutes: i64,
}

fn default_minutes() -> i64 {
    60
}

#[derive(Serialize)]
struct AuthorizeReport {
    token_id: Uuid,
    payload_hash: String,
    expires_in_minutes: i64,
    note: &'static str,
}

/// Issue a one-use individual authorization for exactly the payload this application currently holds. Changing an answer afterwards invalidates it, which is the point.
async fn authorize_submit(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<AuthorizeBody>,
) -> Result<Json<AuthorizeReport>, ApiError> {
    // The same slot a submission takes: issuing a token drives the browser too.
    let slot = workflow::claim_browser_slot(&state.db).await?;
    let inspected = opportunity_core::worker::inspect(&body.url).await;
    slot.release().await;
    let plan = inspected.map_err(|e| ApiError::BadRequest(workflow::worker_message(&e)))?;
    let facts = applications::approved_facts(&state.db).await?;
    let answers = applications::approved_answers(&state.db).await?;
    let mapping = opportunity_domain::map_form(&plan, &facts, &answers);
    // Through the same settling step submit uses, or the token would authorize a different payload than the one that gets sent.
    let mapping = workflow::with_answers(&state.db, id, mapping).await?;
    let hash = workflow::payload_hash(&workflow::payload_for(&mapping));
    let token_id = applications::issue_token(
        &state.db,
        id,
        &body.url,
        &body.source_key,
        &hash,
        body.valid_for_minutes,
    )
    .await?;
    Ok(Json(AuthorizeReport {
        token_id,
        payload_hash: hash,
        expires_in_minutes: body.valid_for_minutes,
        note: "this authorization covers one submission of exactly these answers to exactly this destination",
    }))
}

async fn submit(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<FormBody>,
) -> Result<Json<SubmitReport>, ApiError> {
    let source_key = body.source_key.ok_or_else(|| {
        ApiError::BadRequest("source_key is required so a policy can be matched against it".into())
    })?;
    Ok(Json(
        workflow::submit(&state.db, id, &body.url, &source_key, body.document_path).await?,
    ))
}

#[derive(Debug, Deserialize)]
struct ReviewQuery {
    #[serde(default)]
    application_id: Option<Uuid>,
}

async fn list_review_tasks(
    State(state): State<AppState>,
    Query(params): Query<ReviewQuery>,
) -> Result<Json<Vec<ReviewTask>>, ApiError> {
    Ok(Json(
        applications::list_review_tasks(&state.db, params.application_id).await?,
    ))
}

#[derive(Debug, Deserialize)]
struct AnswerBody {
    answer: String,
}

async fn answer_review_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<AnswerBody>,
) -> Result<Json<ReviewTask>, ApiError> {
    Ok(Json(
        applications::answer_review_task(&state.db, id, &body.answer).await?,
    ))
}

async fn list_policies(State(state): State<AppState>) -> Result<Json<Vec<StoredPolicy>>, ApiError> {
    Ok(Json(applications::list_policies(&state.db).await?))
}

async fn create_policy(
    State(state): State<AppState>,
    Json(policy): Json<NewPolicy>,
) -> Result<(StatusCode, Json<StoredPolicy>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(applications::create_policy(&state.db, policy).await?),
    ))
}

#[derive(Debug, Deserialize)]
struct PolicyStateBody {
    #[serde(default)]
    enabled: bool,
    /// Revoking is permanent and beats enabling, so a revoke request can never accidentally turn a policy on.
    #[serde(default)]
    revoke: bool,
}

async fn set_policy_state(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<PolicyStateBody>,
) -> Result<Json<StoredPolicy>, ApiError> {
    Ok(Json(
        applications::set_policy_state(&state.db, id, body.enabled, body.revoke).await?,
    ))
}

#[derive(Debug, Deserialize)]
struct NotificationQuery {
    #[serde(default)]
    undelivered_only: bool,
}

async fn list_notifications(
    State(state): State<AppState>,
    Query(params): Query<NotificationQuery>,
) -> Result<Json<Vec<Notification>>, ApiError> {
    Ok(Json(
        applications::list_notifications(&state.db, params.undelivered_only).await?,
    ))
}

async fn mark_delivered(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    applications::mark_delivered(&state.db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
