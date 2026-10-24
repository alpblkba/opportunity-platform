//! Routes for profiles, exclusion rules, matching, feedback and the digest.

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use opportunity_core::digest::{self, DigestFormat, NewFeedback, StoredDigest, StoredFeedback};
use opportunity_core::matching::{self, MatchPage, MatchQuery};
use opportunity_core::profiles::{self, NewRule, StoredProfile, StoredRule};
use opportunity_domain::Profile;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ApiError;
use crate::routes::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/profiles", get(list_profiles).post(create_profile))
        .route(
            "/profiles/{id}",
            get(get_profile)
                .patch(update_profile)
                .delete(delete_profile),
        )
        .route("/profiles/seed", post(seed_profiles))
        .route("/rules", get(list_rules).post(create_rule))
        .route("/rules/{id}", axum::routing::delete(delete_rule))
        .route("/matches", get(matches))
        .route("/feedback", post(record_feedback))
        .route("/feedback/sample", get(sample_unseen))
        .route("/digests", get(list_digests))
        .route("/digests/generate", post(generate_digest))
        .route("/export", get(export))
        .route("/vocabulary", get(vocabulary))
}

async fn list_profiles(
    State(state): State<AppState>,
) -> Result<Json<Vec<StoredProfile>>, ApiError> {
    Ok(Json(profiles::list(&state.db).await?))
}

async fn get_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<StoredProfile>, ApiError> {
    Ok(Json(profiles::get(&state.db, id).await?))
}

async fn create_profile(
    State(state): State<AppState>,
    Json(profile): Json<Profile>,
) -> Result<(StatusCode, Json<StoredProfile>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(profiles::create(&state.db, profile).await?),
    ))
}

async fn update_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(profile): Json<Profile>,
) -> Result<Json<StoredProfile>, ApiError> {
    Ok(Json(profiles::update(&state.db, id, profile).await?))
}

async fn delete_profile(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    profiles::delete(&state.db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct SeedReport {
    created: Vec<String>,
}

/// Create the four starting technical tracks if they do not exist. Existing profiles are never overwritten, so this is safe to call again.
async fn seed_profiles(State(state): State<AppState>) -> Result<Json<SeedReport>, ApiError> {
    Ok(Json(SeedReport {
        created: profiles::seed_default_profiles(&state.db).await?,
    }))
}

async fn list_rules(State(state): State<AppState>) -> Result<Json<Vec<StoredRule>>, ApiError> {
    Ok(Json(profiles::list_rules(&state.db).await?))
}

async fn create_rule(
    State(state): State<AppState>,
    Json(rule): Json<NewRule>,
) -> Result<(StatusCode, Json<StoredRule>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(profiles::create_rule(&state.db, rule).await?),
    ))
}

async fn delete_rule(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    profiles::delete_rule(&state.db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct MatchParams {
    profile_id: Uuid,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    /// Show what a rule excluded, with the reason, so the owner can undo it.
    #[serde(default)]
    include_excluded: bool,
}

async fn matches(
    State(state): State<AppState>,
    Query(params): Query<MatchParams>,
) -> Result<Json<MatchPage>, ApiError> {
    let query = MatchQuery {
        q: params.q,
        limit: params.limit.unwrap_or(25),
        include_excluded: params.include_excluded,
    };
    Ok(Json(
        matching::matches_for_profile(
            &state.db,
            params.profile_id,
            &query,
            state.embeddings.as_deref(),
        )
        .await?,
    ))
}

async fn record_feedback(
    State(state): State<AppState>,
    Json(feedback): Json<NewFeedback>,
) -> Result<(StatusCode, Json<StoredFeedback>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(digest::record_feedback(&state.db, feedback).await?),
    ))
}

#[derive(Debug, Deserialize)]
struct SampleParams {
    profile_id: Uuid,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Serialize)]
struct SampleItem {
    id: Uuid,
    canonical_title: String,
    organization: String,
}

/// Draw opportunities the ranking did not surface. Judging only what was shown would measure the ranking against itself, so honest recall checking needs this sample.
async fn sample_unseen(
    State(state): State<AppState>,
    Query(params): Query<SampleParams>,
) -> Result<Json<Vec<SampleItem>>, ApiError> {
    let page = matching::matches_for_profile(
        &state.db,
        params.profile_id,
        &MatchQuery {
            q: None,
            limit: 50,
            include_excluded: false,
        },
        state.embeddings.as_deref(),
    )
    .await?;
    let surfaced: Vec<Uuid> = page.items.iter().map(|i| i.opportunity.id).collect();
    let rows = digest::sample_not_surfaced(&state.db, &surfaced, params.limit.unwrap_or(5)).await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, canonical_title, organization)| SampleItem {
                id,
                canonical_title,
                organization,
            })
            .collect(),
    ))
}

async fn list_digests(State(state): State<AppState>) -> Result<Json<Vec<StoredDigest>>, ApiError> {
    Ok(Json(digest::list(&state.db, 20).await?))
}

#[derive(Debug, Deserialize)]
struct DigestParams {
    profile_id: Uuid,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

fn parse_format(value: Option<&str>) -> Result<DigestFormat, ApiError> {
    Ok(match value.unwrap_or("markdown") {
        "markdown" => DigestFormat::Markdown,
        "csv" => DigestFormat::Csv,
        other => {
            return Err(ApiError::BadRequest(format!(
                "unknown digest format '{other}'"
            )))
        }
    })
}

async fn generate_digest(
    State(state): State<AppState>,
    Query(params): Query<DigestParams>,
) -> Result<Json<StoredDigest>, ApiError> {
    let format = parse_format(params.format.as_deref())?;
    Ok(Json(
        digest::generate(
            &state.db,
            params.profile_id,
            format,
            params.limit.unwrap_or(25),
            state.embeddings.as_deref(),
        )
        .await?,
    ))
}

/// Download the current ranking as a file, so the owner can keep a copy outside the tool.
async fn export(
    State(state): State<AppState>,
    Query(params): Query<DigestParams>,
) -> Result<Response, ApiError> {
    let format = parse_format(params.format.as_deref())?;
    let generated = digest::generate(
        &state.db,
        params.profile_id,
        format,
        params.limit.unwrap_or(100),
        state.embeddings.as_deref(),
    )
    .await?;
    let (content_type, filename) = match format {
        DigestFormat::Markdown => ("text/markdown; charset=utf-8", "opportunities.md"),
        DigestFormat::Csv => ("text/csv; charset=utf-8", "opportunities.csv"),
    };
    Ok((
        [
            (header::CONTENT_TYPE, content_type.to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        generated.body,
    )
        .into_response())
}

#[derive(Serialize)]
struct Vocabulary {
    /// Concept names the matcher understands, offered in the profile editor so a term can be chosen rather than guessed at.
    concepts: Vec<&'static str>,
    job_kinds: Vec<&'static str>,
}

async fn vocabulary() -> Json<Vocabulary> {
    Json(Vocabulary {
        concepts: opportunity_domain::concepts::all_concepts(),
        job_kinds: vec![
            "employment",
            "part_time",
            "internship",
            "working_student",
            "master_thesis",
            "research_assistant",
            "phd",
            "fellowship",
            "other",
        ],
    })
}
