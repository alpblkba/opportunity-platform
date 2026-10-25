//! Authenticated CV upload/download. CV content is never parsed into claims or submitted.
use crate::{error::ApiError, AppState};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use opportunity_core::documents::{self, Document, DocumentError};
use serde::Deserialize;
use uuid::Uuid;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/documents/cv",
            get(list)
                .post(upload)
                .layer(DefaultBodyLimit::max(documents::MAX_CV_BYTES)),
        )
        .route("/documents/{id}/download", get(download))
}

impl From<DocumentError> for ApiError {
    fn from(error: DocumentError) -> Self {
        match error {
            DocumentError::Invalid(message) => Self::BadRequest(message),
            DocumentError::NotFound => Self::NotFound("CV not found".into()),
            other => Self::Internal(other.to_string()),
        }
    }
}

#[derive(Deserialize)]
struct UploadQuery {
    filename: String,
}

async fn upload(
    State(state): State<AppState>,
    Query(query): Query<UploadQuery>,
    body: Bytes,
) -> Result<(StatusCode, Json<Document>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(documents::upload(&state.db, &query.filename, &body).await?),
    ))
}

async fn list(State(state): State<AppState>) -> Result<Json<Vec<Document>>, ApiError> {
    Ok(Json(documents::list(&state.db).await?))
}

async fn download(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let (label, bytes) = documents::download(&state.db, id).await?;
    let extension = if label.to_ascii_lowercase().ends_with(".pdf") {
        "pdf"
    } else {
        "txt"
    };
    let encoded: String = label
        .as_bytes()
        .iter()
        .map(|byte| format!("%{byte:02X}"))
        .collect();
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"cv-{id}.{extension}\"; filename*=UTF-8''{encoded}"),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        bytes,
    )
        .into_response())
}
