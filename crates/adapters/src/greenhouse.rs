use chrono::{DateTime, Utc};
use opportunity_domain::{decode_entities, FetchedPosting, RunOutcome, ScanResult, SourceRunStats};
use serde::Deserialize;

use crate::client::HttpClient;
use crate::location;

pub const HOST: &str = "boards-api.greenhouse.io";

/// Documented job board endpoint. `content=true` is what returns the description body; without it the board returns titles only.
fn endpoint(board_token: &str) -> String {
    format!("https://{HOST}/v1/boards/{board_token}/jobs?content=true")
}

#[derive(Debug, Deserialize)]
struct Board {
    // Deliberately not `default`: a payload whose shape changed must fail to parse and be reported as a schema error, never as a board with no openings.
    jobs: Vec<Job>,
    #[serde(default)]
    meta: Meta,
}

#[derive(Debug, Default, Deserialize)]
struct Meta {
    total: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct Job {
    id: i64,
    title: String,
    absolute_url: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    location: Option<NamedValue>,
    #[serde(default)]
    departments: Vec<NamedValue>,
    #[serde(default)]
    first_published: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    /// Greenhouse publishes this when the employer sets one; it is usually null.
    #[serde(default)]
    application_deadline: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NamedValue {
    #[serde(default)]
    name: Option<String>,
}

/// Parse a board payload. Separated from the request so golden fixtures and stored raw snapshots can be reparsed offline.
pub fn parse(body: &str) -> Result<(Vec<FetchedPosting>, SourceRunStats, bool), serde_json::Error> {
    let board: Board = serde_json::from_str(body)?;
    let postings: Vec<FetchedPosting> = board.jobs.iter().map(to_posting).collect();
    let records_seen = postings.len() as i32;
    let stats = SourceRunStats {
        pages_fetched: 1,
        records_seen,
        total_reported: board.meta.total,
    };
    // Greenhouse returns the whole board in one response and reports its own total, so a mismatch is the one reliable truncation signal this API offers.
    let complete = board.meta.total.map(|t| t == records_seen).unwrap_or(true);
    Ok((postings, stats, complete))
}

fn to_posting(job: &Job) -> FetchedPosting {
    let location_raw = job
        .location
        .as_ref()
        .and_then(|l| l.name.clone())
        .filter(|s| !s.trim().is_empty());
    let (city, country_from_city) = match location_raw.as_deref().and_then(location::city) {
        Some((city, code)) => (Some(city), Some(code)),
        None => (None, None),
    };
    FetchedPosting {
        native_id: job.id.to_string(),
        url: job.absolute_url.clone(),
        // Greenhouse entity-escapes the markup itself, so the payload has to be decoded once before it is HTML at all.
        description_html: job
            .content
            .as_deref()
            .map(decode_entities)
            .filter(|s| !s.trim().is_empty()),
        title: job.title.clone(),
        country_code: location_raw
            .as_deref()
            .and_then(location::country_code)
            .or(country_from_city),
        city,
        remote_raw: location_raw.as_deref().and_then(location::remote_hint),
        department: job
            .departments
            .iter()
            .find_map(|d| d.name.clone())
            .filter(|s| !s.trim().is_empty()),
        employment_type_raw: None,
        posted_at: job
            .first_published
            .as_deref()
            .or(job.updated_at.as_deref())
            .and_then(parse_timestamp),
        supervisor: None,
        contact: None,
        deadline_at: job
            .application_deadline
            .as_deref()
            .and_then(parse_timestamp),
        advertised_state: None,
        location_raw,
    }
}

pub(crate) fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

pub async fn scan(client: &HttpClient, board_token: &str) -> ScanResult {
    let url = endpoint(board_token);
    let response = match client.get_json(&url, &[HOST]).await {
        Ok(response) => response,
        Err(error) => return ScanResult::failed(RunOutcome::HttpError, error.to_string()),
    };
    if let Some(outcome) = crate::status_outcome(response.status) {
        return ScanResult::failed(
            outcome,
            format!("greenhouse board responded {}", response.status),
        );
    }
    match parse(&response.body) {
        Ok((postings, stats, complete)) => {
            let outcome = crate::outcome_for(complete, postings.len());
            let diagnostic = (!complete).then(|| {
                format!(
                    "board reported {:?} jobs but the response carried {}",
                    stats.total_reported, stats.records_seen
                )
            });
            ScanResult {
                outcome,
                postings,
                stats,
                diagnostic,
                raw_url: Some(response.final_url),
                raw_body: Some(response.body),
            }
        }
        Err(error) => ScanResult::failed(
            RunOutcome::SchemaError,
            format!("greenhouse payload did not parse: {error}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/greenhouse-board.json");
    const TRUNCATED: &str = include_str!("../tests/fixtures/greenhouse-board-truncated.json");

    #[test]
    fn golden_fixture_parses_into_typed_postings() {
        let (postings, stats, complete) = parse(FIXTURE).unwrap();
        assert!(complete);
        assert_eq!(stats.records_seen, 3);
        assert_eq!(stats.total_reported, Some(3));
        let firmware = &postings[0];
        assert_eq!(firmware.native_id, "5000000001");
        assert_eq!(firmware.title, "Firmware Engineer, Platform");
        assert_eq!(firmware.city.as_deref(), Some("Munich"));
        assert_eq!(firmware.country_code.as_deref(), Some("DE"));
        assert_eq!(firmware.department.as_deref(), Some("Silicon Platform"));
        assert!(firmware
            .description_html
            .as_deref()
            .unwrap()
            .contains("<p>"));
        assert!(!firmware.is_title_only());
    }

    #[test]
    fn a_row_without_a_description_survives_and_is_marked_title_only() {
        let (postings, _, _) = parse(FIXTURE).unwrap();
        let bare = postings
            .iter()
            .find(|p| p.native_id == "5000000003")
            .unwrap();
        assert_eq!(bare.title, "Electrical Design Engineer");
        assert!(bare.description_html.is_none());
        assert!(bare.is_title_only());
    }

    #[test]
    fn a_total_that_exceeds_the_returned_rows_is_an_incomplete_sweep() {
        let (postings, stats, complete) = parse(TRUNCATED).unwrap();
        assert!(!complete);
        assert_eq!(postings.len(), 1);
        assert_eq!(stats.total_reported, Some(9));
        assert!(!crate::outcome_for(complete, postings.len()).is_complete_sweep());
    }

    #[test]
    fn a_payload_of_the_wrong_shape_is_a_schema_error_not_an_empty_board() {
        assert!(parse(r#"{"jobs": "not an array"}"#).is_err());
        assert!(parse("{}").is_err());
        assert!(parse("[]").is_err());
    }

    #[test]
    fn a_genuinely_empty_board_parses_as_a_verified_empty_sweep() {
        let (postings, _, complete) = parse(r#"{"jobs":[],"meta":{"total":0}}"#).unwrap();
        assert!(postings.is_empty());
        assert!(complete);
        assert_eq!(crate::outcome_for(complete, 0), RunOutcome::OkVerifiedEmpty);
    }
}
