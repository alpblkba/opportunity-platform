use crate::{error::ApiError, AppState};
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use opportunity_core::{applications, product};
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
}
async fn home(State(s): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(product::dashboard(&s.db).await?))
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
struct AnswerBody {
    answer: String,
}
async fn answer(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(b): Json<AnswerBody>,
) -> Result<Json<Value>, ApiError> {
    let task = applications::answer_review_task(&s.db, id, &b.answer).await?;
    sqlx::query("update applications set status='saved',auto_resume=true,updated_at=now() where id=$1 and status in ('needs_review','ready')").bind(task.application_id).execute(&s.db).await?;
    Ok(Json(json!({"ok":true})))
}
