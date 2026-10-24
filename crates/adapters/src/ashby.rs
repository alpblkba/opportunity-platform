use opportunity_domain::{FetchedPosting, RunOutcome, ScanResult, SourceRunStats};
use serde::Deserialize;

use crate::client::HttpClient;
use crate::greenhouse::parse_timestamp;
use crate::location;

pub const HOST: &str = "api.ashbyhq.com";

fn endpoint(board_slug: &str) -> String {
    format!("https://{HOST}/posting-api/job-board/{board_slug}")
}

#[derive(Debug, Deserialize)]
struct Board {
    // Deliberately not `default`: a payload whose shape changed must fail to parse and be reported as a schema error, never as a board with no openings.
    jobs: Vec<Job>,
}

#[derive(Debug, Deserialize)]
struct Job {
    id: String,
    title: String,
    #[serde(rename = "jobUrl", default)]
    job_url: Option<String>,
    #[serde(rename = "applyUrl", default)]
    apply_url: Option<String>,
    #[serde(rename = "descriptionHtml", default)]
    description_html: Option<String>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    department: Option<String>,
    #[serde(rename = "employmentType", default)]
    employment_type: Option<String>,
    #[serde(rename = "publishedAt", default)]
    published_at: Option<String>,
    // The live board sends an explicit null here for some jobs, which is the board declining to say rather than saying no. `default` would only cover a missing key, so this is an Option.
    #[serde(rename = "isRemote", default)]
    is_remote: Option<bool>,
    #[serde(rename = "workplaceType", default)]
    workplace_type: Option<String>,
    #[serde(rename = "isListed", default)]
    is_listed: Option<bool>,
    #[serde(default)]
    address: Option<Address>,
}

#[derive(Debug, Deserialize)]
struct Address {
    #[serde(rename = "postalAddress", default)]
    postal_address: Option<PostalAddress>,
}

#[derive(Debug, Deserialize)]
struct PostalAddress {
    #[serde(rename = "addressCountry", default)]
    address_country: Option<String>,
    #[serde(rename = "addressLocality", default)]
    address_locality: Option<String>,
}

/// Parse a board payload. Only a job the board explicitly marks as not listed is dropped, because that is the board's own statement; a job whose flag the board omits or nulls is kept, since dropping it would lose a real opening.
pub fn parse(body: &str) -> Result<(Vec<FetchedPosting>, SourceRunStats), serde_json::Error> {
    let board: Board = serde_json::from_str(body)?;
    let postings: Vec<FetchedPosting> = board
        .jobs
        .iter()
        .filter(|j| j.is_listed != Some(false))
        .map(to_posting)
        .collect();
    let stats = SourceRunStats {
        pages_fetched: 1,
        records_seen: postings.len() as i32,
        total_reported: None,
    };
    Ok((postings, stats))
}

fn to_posting(job: &Job) -> FetchedPosting {
    let location_raw = job.location.clone().filter(|s| !s.trim().is_empty());
    let postal = job.address.as_ref().and_then(|a| a.postal_address.as_ref());
    // The structured postal address is a stronger statement of place than the display string, so it is consulted first.
    let city = postal.and_then(|p| p.address_locality.clone()).or_else(|| {
        location_raw
            .as_deref()
            .and_then(location::city)
            .map(|(city, _)| city)
    });
    let country_code = postal
        .and_then(|p| p.address_country.as_deref())
        .and_then(location::country_code)
        .or_else(|| location_raw.as_deref().and_then(location::country_code));
    let remote_raw = match job.is_remote {
        Some(true) => Some(
            job.workplace_type
                .clone()
                .unwrap_or_else(|| "Remote".to_owned()),
        ),
        Some(false) => job
            .workplace_type
            .clone()
            .filter(|w| w.eq_ignore_ascii_case("hybrid")),
        // The board said nothing about remote work, so the only evidence left is whatever the location text itself states.
        None => location_raw.as_deref().and_then(location::remote_hint),
    };
    FetchedPosting {
        native_id: job.id.clone(),
        url: job
            .job_url
            .clone()
            .or_else(|| job.apply_url.clone())
            .unwrap_or_else(|| format!("https://jobs.ashbyhq.com/{}", job.id)),
        title: job.title.clone(),
        description_html: job
            .description_html
            .clone()
            .filter(|s| !s.trim().is_empty()),
        country_code,
        city,
        remote_raw,
        department: job.department.clone().filter(|s| !s.trim().is_empty()),
        employment_type_raw: job.employment_type.clone().filter(|s| !s.trim().is_empty()),
        posted_at: job.published_at.as_deref().and_then(parse_timestamp),
        supervisor: None,
        contact: None,
        deadline_at: None,
        advertised_state: None,
        location_raw,
    }
}

pub async fn scan(client: &HttpClient, board_slug: &str) -> ScanResult {
    let url = endpoint(board_slug);
    let response = match client.get_json(&url, &[HOST]).await {
        Ok(response) => response,
        Err(error) => return ScanResult::failed(RunOutcome::HttpError, error.to_string()),
    };
    if let Some(outcome) = crate::status_outcome(response.status) {
        return ScanResult::failed(
            outcome,
            format!("ashby board responded {}", response.status),
        );
    }
    match parse(&response.body) {
        // The posting API returns the whole board in one response and documents no pagination, so a clean parse is a complete inventory.
        Ok((postings, stats)) => ScanResult {
            outcome: crate::outcome_for(true, postings.len()),
            postings,
            stats,
            diagnostic: None,
            raw_url: Some(response.final_url),
            raw_body: Some(response.body),
        },
        Err(error) => ScanResult::failed(
            RunOutcome::SchemaError,
            format!("ashby payload did not parse: {error}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/ashby-board.json");

    #[test]
    fn golden_fixture_parses_into_typed_postings() {
        let (postings, stats) = parse(FIXTURE).unwrap();
        assert_eq!(stats.records_seen, 3);
        let fpga = &postings[0];
        assert_eq!(fpga.title, "FPGA Research Assistant");
        assert_eq!(fpga.city.as_deref(), Some("Sunnyvale"));
        assert_eq!(fpga.country_code.as_deref(), Some("US"));
        assert_eq!(fpga.employment_type_raw.as_deref(), Some("FullTime"));
        assert!(fpga.posted_at.is_some());
    }

    #[test]
    fn an_unlisted_job_is_not_presented_as_an_open_posting() {
        let (postings, _) = parse(FIXTURE).unwrap();
        assert!(postings.iter().all(|p| p.title != "Hidden Draft Role"));
    }

    #[test]
    fn a_remote_job_keeps_the_boards_own_wording() {
        let (postings, _) = parse(FIXTURE).unwrap();
        let remote = postings
            .iter()
            .find(|p| p.native_id == "a0000000-0000-0000-0000-000000000002")
            .unwrap();
        assert_eq!(remote.remote_raw.as_deref(), Some("Remote"));
    }

    #[test]
    fn a_job_whose_board_leaves_remote_unstated_parses_and_stays_unknown() {
        let (postings, stats) = parse(FIXTURE).unwrap();
        assert_eq!(stats.records_seen, 3);
        let unstated = postings
            .iter()
            .find(|p| p.native_id == "a0000000-0000-0000-0000-000000000004")
            .unwrap();
        assert_eq!(unstated.title, "Platform Firmware Engineer");
        assert_eq!(
            unstated.remote_raw, None,
            "an unstated remote flag must stay unknown, not become on-site"
        );
        assert_eq!(unstated.city.as_deref(), Some("Munich"));
    }

    #[test]
    fn a_payload_of_the_wrong_shape_is_a_parse_error_not_an_empty_board() {
        assert!(parse("[]").is_err());
        assert!(parse("{}").is_err());
        assert!(parse(r#"{"apiVersion":"1","postings":[]}"#).is_err());
    }

    #[test]
    fn a_genuinely_empty_board_parses_as_zero_postings() {
        let (postings, stats) = parse(r#"{"apiVersion":"1","jobs":[]}"#).unwrap();
        assert!(postings.is_empty());
        assert_eq!(stats.records_seen, 0);
    }
}
