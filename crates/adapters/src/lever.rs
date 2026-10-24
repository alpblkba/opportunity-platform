use chrono::{DateTime, TimeZone, Utc};
use opportunity_domain::{FetchedPosting, RunOutcome, ScanResult, SourceRunStats};
use serde::Deserialize;

use crate::client::HttpClient;
use crate::location;

pub const HOST: &str = "api.lever.co";

/// Lever serves its postings API in pages. A modest page size keeps each response small, and a hard page cap keeps a misbehaving board from turning one refresh into thousands of requests.
const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 30;

fn endpoint(site_id: &str, skip: usize) -> String {
    format!("https://{HOST}/v0/postings/{site_id}?mode=json&limit={PAGE_SIZE}&skip={skip}")
}

#[derive(Debug, Deserialize)]
struct Posting {
    id: String,
    text: String,
    #[serde(rename = "hostedUrl")]
    hosted_url: Option<String>,
    #[serde(rename = "applyUrl")]
    apply_url: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    categories: Categories,
    #[serde(default)]
    country: Option<String>,
    #[serde(rename = "workplaceType", default)]
    workplace_type: Option<String>,
    #[serde(rename = "createdAt", default)]
    created_at: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
struct Categories {
    #[serde(default)]
    commitment: Option<String>,
    #[serde(default)]
    department: Option<String>,
    #[serde(default)]
    location: Option<String>,
}

/// Parse one page of the postings array. Offline reparse uses the same function on a stored snapshot.
pub fn parse_page(body: &str) -> Result<Vec<FetchedPosting>, serde_json::Error> {
    let postings: Vec<Posting> = serde_json::from_str(body)?;
    Ok(postings.iter().map(to_posting).collect())
}

fn to_posting(posting: &Posting) -> FetchedPosting {
    let location_raw = posting
        .categories
        .location
        .clone()
        .filter(|s| !s.trim().is_empty());
    let city = location_raw
        .as_deref()
        .and_then(location::city)
        .map(|(city, _)| city);
    // Lever states an ISO country code of its own, which is better evidence than anything parsed out of the free-text location.
    let country_code = posting
        .country
        .clone()
        .filter(|c| c.len() == 2)
        .map(|c| c.to_ascii_uppercase())
        .or_else(|| location_raw.as_deref().and_then(location::country_code));
    let remote_raw = posting
        .workplace_type
        .clone()
        .filter(|w| w.eq_ignore_ascii_case("remote") || w.eq_ignore_ascii_case("hybrid"))
        .or_else(|| location_raw.as_deref().and_then(location::remote_hint));
    FetchedPosting {
        native_id: posting.id.clone(),
        url: posting
            .hosted_url
            .clone()
            .or_else(|| posting.apply_url.clone())
            .unwrap_or_else(|| format!("https://jobs.lever.co/{}", posting.id)),
        title: posting.text.clone(),
        description_html: posting.description.clone().filter(|s| !s.trim().is_empty()),
        country_code,
        city,
        remote_raw,
        department: posting
            .categories
            .department
            .clone()
            .filter(|s| !s.trim().is_empty()),
        employment_type_raw: posting
            .categories
            .commitment
            .clone()
            .filter(|s| !s.trim().is_empty()),
        posted_at: posting.created_at.and_then(from_millis),
        supervisor: None,
        contact: None,
        deadline_at: None,
        advertised_state: None,
        location_raw,
    }
}

fn from_millis(millis: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_millis_opt(millis).single()
}

pub async fn scan(client: &HttpClient, site_id: &str) -> ScanResult {
    let mut postings: Vec<FetchedPosting> = Vec::new();
    let mut pages_fetched = 0i32;
    let mut last_body = None;
    let mut last_url = None;

    for page in 0..MAX_PAGES {
        let url = endpoint(site_id, page * PAGE_SIZE);
        let response = match client.get_json(&url, &[HOST]).await {
            Ok(response) => response,
            Err(error) => {
                return partial_or_failure(
                    postings,
                    pages_fetched,
                    RunOutcome::HttpError,
                    error.to_string(),
                )
            }
        };
        if let Some(outcome) = crate::status_outcome(response.status) {
            let detail = format!("lever site responded {} on page {}", response.status, page);
            return partial_or_failure(postings, pages_fetched, outcome, detail);
        }
        let page_postings = match parse_page(&response.body) {
            Ok(page_postings) => page_postings,
            Err(error) => {
                let detail = format!("lever payload did not parse on page {page}: {error}");
                return partial_or_failure(
                    postings,
                    pages_fetched,
                    RunOutcome::SchemaError,
                    detail,
                );
            }
        };
        pages_fetched += 1;
        let page_len = page_postings.len();
        postings.extend(page_postings);
        last_url = Some(response.final_url);
        last_body = Some(response.body);
        if page_len < PAGE_SIZE {
            let stats = SourceRunStats {
                pages_fetched,
                records_seen: postings.len() as i32,
                total_reported: None,
            };
            let outcome = crate::outcome_for(true, postings.len());
            return ScanResult {
                outcome,
                postings,
                stats,
                diagnostic: None,
                raw_body: last_body,
                raw_url: last_url,
            };
        }
    }

    // The page cap was reached with every page still full, so the board holds more than was read. Whatever was read is kept, but the sweep is not a complete inventory and must not close anything.
    let stats = SourceRunStats {
        pages_fetched,
        records_seen: postings.len() as i32,
        total_reported: None,
    };
    ScanResult {
        outcome: RunOutcome::Partial,
        postings,
        stats,
        diagnostic: Some(format!("stopped at the {MAX_PAGES} page cap with a full final page; the board has more postings than were read")),
        raw_body: last_body,
        raw_url: last_url,
    }
}

/// A failure after some pages already succeeded keeps those records and reports `partial`, because the records that were read are real and the ones that were not are unknown rather than gone.
fn partial_or_failure(
    postings: Vec<FetchedPosting>,
    pages_fetched: i32,
    outcome: RunOutcome,
    diagnostic: String,
) -> ScanResult {
    if postings.is_empty() {
        return ScanResult::failed(outcome, diagnostic);
    }
    let stats = SourceRunStats {
        pages_fetched,
        records_seen: postings.len() as i32,
        total_reported: None,
    };
    ScanResult {
        outcome: RunOutcome::Partial,
        postings,
        stats,
        diagnostic: Some(format!(
            "{diagnostic}; keeping the {pages_fetched} pages already read"
        )),
        raw_body: None,
        raw_url: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/lever-postings.json");

    #[test]
    fn golden_fixture_parses_into_typed_postings() {
        let postings = parse_page(FIXTURE).unwrap();
        assert_eq!(postings.len(), 3);
        let thesis = &postings[1];
        assert_eq!(thesis.title, "Masterarbeit: Speicherhierarchien für RISC-V");
        assert_eq!(thesis.country_code.as_deref(), Some("DE"));
        assert_eq!(thesis.city.as_deref(), Some("Karlsruhe"));
        assert_eq!(thesis.employment_type_raw.as_deref(), Some("Intern"));
    }

    #[test]
    fn the_sites_own_country_code_wins_over_free_text_parsing() {
        let postings = parse_page(FIXTURE).unwrap();
        let remote = postings
            .iter()
            .find(|p| p.native_id == "c0000000-0000-0000-0000-000000000003")
            .unwrap();
        assert_eq!(remote.country_code.as_deref(), Some("US"));
        assert_eq!(remote.remote_raw.as_deref(), Some("remote"));
        assert!(remote.is_title_only());
    }

    #[test]
    fn a_posting_without_a_hosted_url_still_gets_a_usable_link() {
        let postings = parse_page(FIXTURE).unwrap();
        assert!(postings.iter().all(|p| p.url.starts_with("https://")));
    }

    #[test]
    fn a_malformed_payload_is_a_parse_error_not_an_empty_site() {
        assert!(parse_page("{\"not\": \"an array\"}").is_err());
    }
}
