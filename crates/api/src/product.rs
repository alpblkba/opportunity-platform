use crate::{error::ApiError, AppState};
use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use opportunity_core::{applications, preferences, product};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/home", get(home))
        .route("/home/profile", post(profile))
        .route("/home/authorize", post(authorize))
        .route("/home/apply/{id}", post(apply))
        .route("/home/answer/{id}", post(answer))
        .route("/home/pause", post(pause))
        .route("/home/mark/{id}", post(mark))
        .route("/home/dismiss/{id}", post(dismiss))
        .route("/home/block-employer", post(block_employer))
        .route("/home/visited", post(visited))
        .route("/home/track-external", post(track_external))
        .route(
            "/home/notifications",
            get(notification_preferences).post(set_notification_preferences),
        )
}
async fn home(
    State(s): State<AppState>,
    Query(filters): Query<product::Filters>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(product::dashboard_with(&s.db, &filters).await?))
}
#[derive(Deserialize)]
struct MarkBody {
    mark: String,
    on: bool,
}
async fn mark(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<MarkBody>,
) -> Result<Json<Value>, ApiError> {
    preferences::set_mark(&s.db, id, &b.mark, b.on).await?;
    Ok(Json(json!({"ok":true})))
}
async fn dismiss(State(s): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!(preferences::dismiss(&s.db, id).await?)))
}
#[derive(Deserialize)]
struct EmployerBody {
    employer: String,
}
async fn block_employer(
    State(s): State<AppState>,
    Json(b): Json<EmployerBody>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!(
        preferences::block_employer(&s.db, &b.employer).await?
    )))
}
async fn track_external(
    State(s): State<AppState>,
    Json(job): Json<product::ExternalJob>,
) -> Result<Json<Value>, ApiError> {
    let (application_id, duplicate) = product::track_external(&s.db, job).await?;
    Ok(Json(
        json!({"application_id": application_id, "duplicate": duplicate}),
    ))
}
async fn visited(State(s): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"previous_visit_at": preferences::record_visit(&s.db).await?}),
    ))
}
async fn notification_preferences(State(s): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!(
        preferences::notification_preferences(&s.db).await?
    )))
}
async fn set_notification_preferences(
    State(s): State<AppState>,
    Json(b): Json<preferences::NotificationPreferences>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!(
        preferences::set_notification_preferences(&s.db, b).await?
    )))
}
#[derive(Deserialize)]
struct ProfileBody {
    profile_id: Uuid,
}
async fn profile(
    State(s): State<AppState>,
    Json(b): Json<ProfileBody>,
) -> Result<Json<Value>, ApiError> {
    product::select(&s.db, b.profile_id).await?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
struct AuthorizationBody {
    profile_id: Uuid,
    source_key: String,
}
async fn authorize(
    State(s): State<AppState>,
    Json(b): Json<AuthorizationBody>,
) -> Result<Json<Value>, ApiError> {
    product::authorize_source(&s.db, &b.source_key, b.profile_id).await?;
    Ok(Json(json!({"ok":true})))
}
async fn apply(State(s): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({"id":product::enqueue(&s.db,id).await?})))
}
#[derive(Deserialize)]
struct PauseBody {
    paused: bool,
}
async fn pause(
    State(s): State<AppState>,
    Json(b): Json<PauseBody>,
) -> Result<Json<Value>, ApiError> {
    product::pause_submissions(&s.db, b.paused).await?;
    Ok(Json(json!({"ok":true})))
}
#[derive(Deserialize)]
struct AnswerBody {
    answer: String,
}
async fn answer(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<AnswerBody>,
) -> Result<Json<Value>, ApiError> {
    let task = applications::answer_review_task(&s.db, id, &b.answer).await?;
    applications::resume_after_answers(&s.db, task.application_id).await?;
    Ok(Json(json!({"ok":true})))
}
