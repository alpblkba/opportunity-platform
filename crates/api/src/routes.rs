use std::path::PathBuf;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use opportunity_adapters::HttpClient;
use opportunity_core::ingest::{self, IngestSummary};
use opportunity_core::query::{self, OpportunityQuery};
use opportunity_core::sources::{self, NewSource, Source, SourceUpdate};
use opportunity_core::Db;
use serde::{Deserialize, Serialize};
use tower_http::services::{ServeDir, ServeFile};
use uuid::Uuid;

use crate::error::ApiError;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub http: HttpClient,
    /// Present only when an embedding provider is configured. `None` means the matcher runs lexically and says so.
    pub embeddings: Option<std::sync::Arc<opportunity_core::embeddings::EmbeddingProvider>>,
    /// Directory of the built frontend. When present the API serves the UI too, so one process and one port is the whole application.
    pub web_dist: Option<PathBuf>,
    /// How the service decides who is calling. Every route that touches personal data runs through it.
    pub access: crate::access::AccessMode,
    /// Whether the session cookie may carry `Secure` and the `__Host-` prefix. True everywhere the
    /// service is reached over HTTPS, which is everywhere except a loopback development server.
    pub secure_cookies: bool,
    /// Failed login attempts, in memory. Shared so every request sees the same counts.
    pub logins: std::sync::Arc<crate::auth::LoginLimiter>,
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/ops/status", get(ops_status))
        .route("/stats", get(stats))
        .route("/coverage", get(coverage))
        .route("/opportunities", get(list_opportunities))
        .route("/opportunities/{id}", get(get_opportunity))
        .route("/sources", get(list_sources).post(create_source))
        .route("/sources/{id}", patch(update_source).delete(delete_source))
        .route("/sources/{id}/validate", post(validate_source))
        .route("/sources/{id}/refresh", post(refresh_source))
        .route("/source-runs", get(list_source_runs))
        .route("/refresh", post(refresh_all))
        .merge(crate::personalization::router())
        .merge(crate::applications::router())
        .merge(crate::documents::router())
        .merge(crate::product::router());

    // Everything under /api touches personal data, so the identity check wraps all of it in one place rather than route by route, where one forgotten route would be a leak.
    let api = api.route_layer(axum::middleware::from_fn_with_state(
        state.clone(),
        crate::access::require_identity,
    ));

    // The three routes an unauthenticated browser may reach, merged after the layer above rather
    // than excepted from it, so there is no list of exemptions to get wrong.
    let api = api.merge(crate::auth::router());

    // Liveness and readiness stay open because a container orchestrator must be able to probe them. They return a status and a version and nothing else.
    let mut app = Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .nest("/api", api);

    if let Some(dist) = state.web_dist.clone() {
        let index = dist.join("index.html");
        // Unknown paths fall back to index.html so the single-page app owns its own routing, which
        // is what makes /login a URL someone can open, reload and bookmark.
        // `fallback` rather than `not_found_service`: the latter keeps the 404 status while serving
        // the page, so /login rendered but answered 404 to anything that was reading the status -
        // a monitor, a link checker, or anyone testing that a deep link works.
        let files = ServeDir::new(dist).fallback(ServeFile::new(index));
        // Where a native session is the boundary, the bundle is public: the login page is part of
        // it, and it has to load before anyone can log in. It carries no personal data - every
        // answer comes from /api, which is protected. Where Cloudflare alone is the boundary, the
        // bundle stays behind it exactly as before, so that configuration is unchanged and remains
        // the rollback.
        app = if state.access.uses_sessions() {
            app.fallback_service(axum::routing::any_service(files))
        } else {
            app.fallback_service(axum::routing::any_service(files).layer(
                axum::middleware::from_fn_with_state(
                    state.clone(),
                    crate::access::require_identity,
                ),
            ))
        };
    }

    // Applies to every unsafe method on every route, including the login form itself.
    app.layer(axum::middleware::from_fn(crate::auth::require_same_origin))
        .with_state(state)
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Readiness is a real database round trip, not a process-is-alive check, because the UI is useless without the database.
async fn ready(State(state): State<AppState>) -> Result<Json<Health>, ApiError> {
    sqlx::query_scalar::<_, i32>("select 1")
        .fetch_one(&state.db)
        .await?;
    Ok(Json(Health {
        status: "ready",
        version: env!("CARGO_PKG_VERSION"),
    }))
}

/// Behind the same identity check as every other `/api` route: it names failing employers and counts.
async fn ops_status(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(opportunity_core::ops::status(&state.db).await?))
}

async fn stats(State(state): State<AppState>) -> Result<Json<query::Stats>, ApiError> {
    Ok(Json(query::stats(&state.db).await?))
}

async fn coverage(
    State(state): State<AppState>,
) -> Result<Json<Vec<query::CoverageRow>>, ApiError> {
    Ok(Json(query::coverage(&state.db).await?))
}

async fn list_opportunities(
    State(state): State<AppState>,
    Query(params): Query<OpportunityQuery>,
) -> Result<Json<query::OpportunityPage>, ApiError> {
    Ok(Json(query::list_opportunities(&state.db, &params).await?))
}

async fn get_opportunity(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<query::OpportunityDetail>, ApiError> {
    query::get_opportunity(&state.db, id)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("no opportunity with id {id}")))
}

async fn list_sources(State(state): State<AppState>) -> Result<Json<Vec<Source>>, ApiError> {
    Ok(Json(sources::list(&state.db).await?))
}

async fn create_source(
    State(state): State<AppState>,
    Json(body): Json<NewSource>,
) -> Result<(StatusCode, Json<Source>), ApiError> {
    let source = sources::create(&state.db, body).await?;
    Ok((StatusCode::CREATED, Json(source)))
}

async fn update_source(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<SourceUpdate>,
) -> Result<Json<Source>, ApiError> {
    Ok(Json(sources::update(&state.db, id, body).await?))
}

async fn delete_source(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    sources::delete(&state.db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct ValidationReport {
    source_key: String,
    reachable: bool,
    outcome: String,
    records_seen: i32,
    complete: bool,
    diagnostic: Option<String>,
}

/// Registration check: one read-only request, nothing persisted. This is what turns a typo in a board token into a visible error instead of a source that silently reports zero jobs forever.
async fn validate_source(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ValidationReport>, ApiError> {
    let source = sources::get(&state.db, id).await?;
    let endpoint = typed_adapter_endpoint(&source)?;
    endpoint.validate().map_err(ApiError::BadRequest)?;
    let scan = opportunity_adapters::scan(&state.http, &endpoint).await;
    Ok(Json(ValidationReport {
        source_key: source.key,
        reachable: scan.outcome.is_healthy()
            || scan.outcome == opportunity_domain::RunOutcome::Partial,
        outcome: scan.outcome.as_str().to_owned(),
        records_seen: scan.stats.records_seen,
        complete: scan.outcome.is_complete_sweep(),
        diagnostic: scan.diagnostic,
    }))
}

async fn refresh_source(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<IngestSummary>, ApiError> {
    // The durable lock, not an in-process one: a hand-triggered scan has to exclude the background sweep and a browser run, which an in-process flag could not see.
    let Some(lease) = opportunity_core::scheduler::WorkLease::try_acquire(&state.db).await? else {
        return Err(ApiError::Conflict(
            "a refresh or a browser run is already in progress".into(),
        ));
    };
    let source = sources::get(&state.db, id).await;
    let summary = match source {
        Ok(source) => scan_and_ingest(&state, &source).await,
        Err(error) => Err(error.into()),
    };
    lease.release().await;
    Ok(Json(summary?))
}

#[derive(Debug, Deserialize)]
struct RefreshParams {
    /// Refresh every enabled source with an adapter, rather than only the ones their cadence says are due.
    #[serde(default)]
    all: bool,
}

#[derive(Serialize)]
struct RefreshReport {
    sources_attempted: usize,
    summaries: Vec<IngestSummary>,
}

async fn refresh_all(
    State(state): State<AppState>,
    Query(params): Query<RefreshParams>,
) -> Result<Json<RefreshReport>, ApiError> {
    let Some(lease) = opportunity_core::scheduler::WorkLease::try_acquire(&state.db).await? else {
        return Err(ApiError::Conflict(
            "a refresh or a browser run is already in progress".into(),
        ));
    };
    let report = refresh_all_holding_the_machine(&state, params).await;
    lease.release().await;
    report
}

async fn refresh_all_holding_the_machine(
    state: &AppState,
    params: RefreshParams,
) -> Result<Json<RefreshReport>, ApiError> {
    let due = if params.all {
        sources::list(&state.db)
            .await?
            .into_iter()
            .filter(|s| {
                s.enabled
                    && s.typed_endpoint()
                        .map(|e| e.kind().has_adapter())
                        .unwrap_or(false)
            })
            .collect()
    } else {
        opportunity_core::scheduler::due_now(&state.db).await?
    };
    let mut summaries = Vec::with_capacity(due.len());
    for source in &due {
        summaries.push(scan_and_ingest(state, source).await?);
    }
    Ok(Json(RefreshReport {
        sources_attempted: due.len(),
        summaries,
    }))
}

async fn scan_and_ingest(state: &AppState, source: &Source) -> Result<IngestSummary, ApiError> {
    let endpoint = typed_adapter_endpoint(source)?;
    let scan = opportunity_adapters::scan(&state.http, &endpoint).await;
    let outcome = scan.outcome;
    let summary = ingest::ingest(&state.db, source, scan).await?;
    // A hand-triggered scan advances the schedule exactly as a scheduled one does, so refreshing by hand does not leave the source immediately due again.
    opportunity_core::scheduler::record_schedule(&state.db, source, outcome).await?;
    Ok(summary)
}

/// Refuse to scan a source kind that has no adapter. Scanning one and recording the refusal would label a manual watch entry as blocked, which claims the source turned us away when nothing was ever asked of it.
fn typed_adapter_endpoint(source: &Source) -> Result<opportunity_domain::SourceEndpoint, ApiError> {
    let endpoint = source
        .typed_endpoint()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;
    if !endpoint.kind().has_adapter() {
        return Err(ApiError::BadRequest(format!(
            "source kind '{}' has no adapter in this release; it is a manual watch entry whose link you follow by hand",
            endpoint.kind()
        )));
    }
    Ok(endpoint)
}

#[derive(Debug, Deserialize)]
struct RunQuery {
    #[serde(default)]
    source_id: Option<Uuid>,
    #[serde(default)]
    limit: Option<i64>,
}

async fn list_source_runs(
    State(state): State<AppState>,
    Query(params): Query<RunQuery>,
) -> Result<Json<Vec<query::SourceRunRow>>, ApiError> {
    Ok(Json(
        query::source_runs(&state.db, params.source_id, params.limit.unwrap_or(50)).await?,
    ))
}
