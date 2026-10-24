use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use opportunity_core::profiles::ProfileError;
use opportunity_core::sources::SourceError;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ApiErrorBody {
    pub error: String,
    pub detail: String,
}

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    NotFound(String),
    Conflict(String),
    /// Nobody is authenticated. The caller should log in.
    Unauthorized,
    /// Somebody is authenticated, and may not do this. A different answer to a different question,
    /// and conflating the two is how a logged-in user gets bounced to a login page for ever.
    Forbidden(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error, detail) = match self {
            Self::BadRequest(detail) => (StatusCode::BAD_REQUEST, "bad_request", detail),
            Self::NotFound(detail) => (StatusCode::NOT_FOUND, "not_found", detail),
            Self::Conflict(detail) => (StatusCode::CONFLICT, "conflict", detail),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "this service is private and requires an authorized identity".to_owned(),
            ),
            Self::Forbidden(detail) => (StatusCode::FORBIDDEN, "forbidden", detail),
            // The only case whose detail is for the operator rather than the caller. A database
            // error carries constraint names, column names and sometimes fragments of a connection
            // string; it goes to the log, and the caller is told that something failed.
            Self::Internal(detail) => {
                tracing::error!(detail = %detail, "a request failed");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "something went wrong here. The details are in this server's log.".to_owned(),
                )
            }
        };
        (
            status,
            Json(ApiErrorBody {
                error: error.to_owned(),
                detail,
            }),
        )
            .into_response()
    }
}

impl From<SourceError> for ApiError {
    fn from(error: SourceError) -> Self {
        match error {
            SourceError::Invalid(detail) => Self::BadRequest(detail),
            SourceError::NotFound(id) => Self::NotFound(format!("no source with id {id}")),
            // The constraint being violated is the useful part; the database's own message names
            // columns, types and sometimes the value, and none of that belongs in a browser.
            SourceError::Database(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                tracing::warn!(detail = %db, "a unique constraint refused a write");
                Self::Conflict("a source with that key already exists".into())
            }
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<ProfileError> for ApiError {
    fn from(error: ProfileError) -> Self {
        match error {
            ProfileError::Invalid(detail) => Self::BadRequest(detail),
            ProfileError::NotFound(id) => Self::NotFound(format!("no profile with id {id}")),
            ProfileError::Database(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                tracing::warn!(detail = %db, "a unique constraint refused a write");
                Self::Conflict("a profile with that key already exists".into())
            }
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        Self::Internal(error.to_string())
    }
}
