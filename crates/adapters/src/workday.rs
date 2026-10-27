//! Workday career-site adapter.
//!
//! The CXS job-search shape this reads is an undocumented internal interface of a tenant's own career site, not a licensed public candidate API. It can change without notice, so every response is treated as untrusted and every sweep states its own completeness. The listing search uses a read-only POST; it never submits an application.

use opportunity_domain::{FetchedPosting, RunOutcome, ScanResult, SourceRunStats};
use serde::Deserialize;

use crate::client::HttpClient;
use crate::location;

pub const HOST_SUFFIX: &str = "myworkdayjobs.com";

/// Workday rejects a larger page than this on most tenants, and a small page keeps each request cheap on an interface that is not ours to strain.
const PAGE_SIZE: usize = 20;
/// Hard ceiling on requests per sweep. A tenant with more postings than this reports `partial` rather than being hammered.
const MAX_PAGES: usize = 25;

fn endpoint(tenant: &str, instance: &str, site: &str) -> String {
    format!("https://{tenant}.{instance}.{HOST_SUFFIX}/wday/cxs/{tenant}/{site}/jobs")
}

#[derive(Debug, Deserialize)]
struct Page {
    // Deliberately not `default`: a payload whose shape changed must fail to parse and be reported as a schema error, never as a tenant with no openings.
    #[serde(rename = "jobPostings")]
    job_postings: Vec<Posting>,
    #[serde(default)]
    total: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct Posting {
    title: String,
    #[serde(rename = "externalPath")]
    external_path: String,
    #[serde(rename = "locationsText", default)]
    locations_text: Option<String>,
    /// Workday puts the requisition identifier here. It is the only stable native id the list response offers.
    #[serde(rename = "bulletFields", default)]
    bullet_fields: Vec<String>,
}

/// Parse one page. Separated from the request so the pagination-hazard fixtures and stored snapshots can be replayed offline.
pub fn parse_page(
    body: &str,
    tenant: &str,
    instance: &str,
) -> Result<(Vec<FetchedPosting>, Option<i32>), serde_json::Error> {
    let page: Page = serde_json::from_str(body)?;
    let base = format!("https://{tenant}.{instance}.{HOST_SUFFIX}");
    let postings = page
        .job_postings
        .iter()
        .map(|p| to_posting(p, &base))
        .collect();
    Ok((postings, page.total))
}

fn to_posting(posting: &Posting, base: &str) -> FetchedPosting {
    let location_raw = posting
        .locations_text
        .clone()
        .filter(|s| !s.trim().is_empty());
    let (city, country_from_city) = match location_raw.as_deref().and_then(location::city) {
        Some((city, code)) => (Some(city), Some(code)),
        None => (None, None),
    };
    FetchedPosting {
        // The requisition id is preferred because it survives a title edit; the path is the fallback when the tenant publishes no bullet fields.
        native_id: posting
            .bullet_fields
            .iter()
            .find(|f| !f.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| posting.external_path.clone()),
        url: format!("{base}{}", posting.external_path),
        title: posting.title.clone(),
        // The CXS list response carries no description body. Fetching each job's detail page would be a per-posting request against the same undocumented interface, so records stay title-only and are flagged as such.
        description_html: None,
        country_code: location_raw
            .as_deref()
            .and_then(location::country_code)
            .or(country_from_city),
        city,
        remote_raw: location_raw.as_deref().and_then(location::remote_hint),
        department: None,
        employment_type_raw: None,
        // The list response states age as relative wording such as "Posted 5 Days Ago", which is not a date. Turning that into one would invent precision the source never gave, so it is read as unknown and the field is not deserialized at all.
        posted_at: None,
        supervisor: None,
        contact: None,
        deadline_at: None,
        advertised_state: None,
        location_raw,
    }
}

/// Decide the outcome of a completed sweep from what the pages actually delivered.
///
/// This is the whole point of the adapter. Workday pagination fails in ways that look like success: a middle page can come back empty, the reported total can disagree with what was served, a later page can be short before the total is reached, and the same requisition can appear on two pages. Each of those means the inventory was not fully observed, so the sweep must be `partial` and must never be allowed to close a posting.
pub fn assess(
    observed: usize,
    unique: usize,
    total_reported: Option<i32>,
    hazard: Option<&str>,
) -> (RunOutcome, Option<String>) {
    if let Some(reason) = hazard {
        return (RunOutcome::Partial, Some(reason.to_owned()));
    }
    match total_reported {
        Some(total) if total as usize != unique => (
            RunOutcome::Partial,
            Some(format!(
                "tenant reported {total} postings but the sweep read {unique} distinct records"
            )),
        ),
        _ if observed != unique => (
            RunOutcome::Partial,
            Some(format!(
                "{} duplicate requisition identifiers appeared across pages",
                observed - unique
            )),
        ),
        Some(0) => (RunOutcome::OkVerifiedEmpty, None),
        _ if unique == 0 => (RunOutcome::OkVerifiedEmpty, None),
        _ => (RunOutcome::OkComplete, None),
    }
}

/// Workday reports the total on the first page and often 0 on the pages after it, so the first
/// non-zero report stands and a later 0 means nothing. A 0 beside records is no total at all. A later,
/// different non-zero total means the inventory moved during the sweep, which is a partial sweep.
fn reconcile_total(
    known: Option<i32>,
    page: Option<i32>,
    page_len: usize,
) -> (Option<i32>, Option<String>) {
    let page = page.filter(|t| *t > 0 || page_len == 0);
    match (known, page) {
        (None, page) => (page, None),
        (Some(first), Some(t)) if t != first && t != 0 => (
            known,
            Some(format!(
                "the reported total changed from {first} to {t} during the sweep"
            )),
        ),
        _ => (known, None),
    }
}

pub async fn scan(client: &HttpClient, tenant: &str, instance: &str, site: &str) -> ScanResult {
    let url = endpoint(tenant, instance, site);
    let mut postings: Vec<FetchedPosting> = Vec::new();
    let mut pages_fetched = 0i32;
    let mut total_reported = None;
    let mut hazard: Option<String> = None;
    let mut last_body = None;

    for page in 0..MAX_PAGES {
        let body = serde_json::json!({
            "appliedFacets": {},
            "limit": PAGE_SIZE,
            "offset": page * PAGE_SIZE,
            "searchText": ""
        });
        let response = match client.post_json(&url, body, HOST_SUFFIX).await {
            Ok(response) => response,
            Err(error) => {
                return partial_or_failure(
                    postings,
                    pages_fetched,
                    total_reported,
                    RunOutcome::HttpError,
                    error.to_string(),
                )
            }
        };
        if let Some(outcome) = crate::status_outcome(response.status) {
            let detail = format!(
                "workday tenant responded {} on page {page}",
                response.status
            );
            return partial_or_failure(postings, pages_fetched, total_reported, outcome, detail);
        }
        let (mut page_postings, page_total) = match parse_page(&response.body, tenant, instance) {
            Ok(parsed) => parsed,
            Err(error) => {
                let detail = format!("workday payload did not parse on page {page}: {error}");
                return partial_or_failure(
                    postings,
                    pages_fetched,
                    total_reported,
                    RunOutcome::SchemaError,
                    detail,
                );
            }
        };
        for posting in &mut page_postings {
            let origin = format!("https://{tenant}.{instance}.{HOST_SUFFIX}");
            if let Some(path) = posting.url.strip_prefix(&origin) {
                posting.url = format!("{origin}/en-US/{site}{path}");
            }
        }
        pages_fetched += 1;
        let page_len = page_postings.len();
        let (total, changed) = reconcile_total(total_reported, page_total, page_len);
        total_reported = total;
        if let Some(reason) = changed {
            hazard.get_or_insert(reason);
        }
        postings.extend(page_postings);
        last_body = Some(response.body);

        let expected_more = total_reported
            .map(|t| postings.len() < t as usize)
            .unwrap_or(page_len == PAGE_SIZE);
        if page_len == 0 && expected_more {
            hazard = Some(format!(
                "page {page} came back empty while the tenant still reported more postings"
            ));
            break;
        }
        if page_len < PAGE_SIZE {
            if expected_more {
                hazard = Some(format!(
                    "page {page} returned {page_len} of {PAGE_SIZE} records before the reported total was reached"
                ));
            }
            break;
        }
        if page + 1 == MAX_PAGES {
            hazard = Some(format!(
                "stopped at the {MAX_PAGES} page cap with a full final page"
            ));
        }
    }

    let observed = postings.len();
    let mut seen: Vec<&str> = Vec::with_capacity(observed);
    for posting in &postings {
        if !seen.contains(&posting.native_id.as_str()) {
            seen.push(&posting.native_id);
        }
    }
    let unique = seen.len();
    let (outcome, diagnostic) = assess(observed, unique, total_reported, hazard.as_deref());
    ScanResult {
        outcome,
        stats: SourceRunStats {
            pages_fetched,
            records_seen: observed as i32,
            total_reported,
        },
        postings,
        diagnostic,
        raw_url: Some(url),
        raw_body: last_body,
    }
}

fn partial_or_failure(
    postings: Vec<FetchedPosting>,
    pages_fetched: i32,
    total_reported: Option<i32>,
    outcome: RunOutcome,
    diagnostic: String,
) -> ScanResult {
    if postings.is_empty() {
        return ScanResult::failed(outcome, diagnostic);
    }
    let stats = SourceRunStats {
        pages_fetched,
        records_seen: postings.len() as i32,
        total_reported,
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

    const PAGE_FULL: &str = include_str!("../tests/fixtures/workday-page-0.json");
    const PAGE_BLANK_MIDDLE: &str =
        include_str!("../tests/fixtures/workday-page-blank-middle.json");
    const PAGE_TRUNCATED: &str = include_str!("../tests/fixtures/workday-page-truncated.json");
    const PAGE_DUPLICATE: &str = include_str!("../tests/fixtures/workday-page-duplicate-id.json");
    const PAGE_NO_DESCRIPTION: &str =
        include_str!("../tests/fixtures/workday-page-no-description.json");

    #[test]
    fn the_first_reported_total_stands_and_later_zeroes_are_the_known_quirk() {
        // Marvell, 2026-10-27: 343 on the first page, 0 on every later one, 343 records read.
        let mut known = None;
        for (page_total, len) in [(Some(343), 20), (Some(0), 20), (Some(0), 3)] {
            let (total, changed) = reconcile_total(known, page_total, len);
            assert!(changed.is_none());
            known = total;
        }
        assert_eq!(known, Some(343));
        assert_eq!(assess(343, 343, known, None).0, RunOutcome::OkComplete);
        // A zero beside records is no total; a zero on an empty first page is a verified empty board.
        assert_eq!(reconcile_total(None, Some(0), 20).0, None);
        assert_eq!(reconcile_total(None, Some(0), 0).0, Some(0));
        // A different non-zero total later means the inventory moved: partial, never complete.
        let (total, changed) = reconcile_total(Some(343), Some(350), 20);
        assert_eq!(total, Some(343));
        assert!(changed.unwrap().contains("changed from 343 to 350"));
        // The old behaviour, a later 0 overwriting the first total, is what made complete sweeps partial.
        assert_eq!(assess(343, 343, Some(0), None).0, RunOutcome::Partial);
    }

    fn parse(body: &str) -> (Vec<FetchedPosting>, Option<i32>) {
        parse_page(body, "examplechip", "wd3").unwrap()
    }

    #[test]
    fn a_full_page_parses_into_typed_postings_with_tenant_absolute_urls() {
        let (postings, total) = parse(PAGE_FULL);
        assert_eq!(total, Some(42));
        assert_eq!(postings.len(), 3);
        assert_eq!(postings[0].native_id, "R-100001");
        assert_eq!(postings[0].title, "Senior Firmware Engineer");
        assert_eq!(postings[0].url, "https://examplechip.wd3.myworkdayjobs.com/en-US/careers/job/Munich/Senior-Firmware-Engineer_R-100001");
        assert_eq!(postings[0].city.as_deref(), Some("Munich"));
        assert_eq!(postings[0].country_code.as_deref(), Some("DE"));
        assert!(
            postings[0].posted_at.is_none(),
            "relative wording such as 'Posted 5 Days Ago' is not a date"
        );
    }

    /// Hazard 1 of 5: a middle page comes back empty while the tenant still claims more postings.
    #[test]
    fn a_blank_middle_page_makes_the_sweep_partial() {
        let (page0, total) = parse(PAGE_FULL);
        let (blank, _) = parse(PAGE_BLANK_MIDDLE);
        assert!(blank.is_empty());
        let (outcome, diagnostic) = assess(
            page0.len(),
            page0.len(),
            total,
            Some("page 1 came back empty while the tenant still reported more postings"),
        );
        assert_eq!(outcome, RunOutcome::Partial);
        assert!(!outcome.is_complete_sweep());
        assert!(diagnostic.unwrap().contains("empty"));
    }

    /// Hazard 2 of 5: the reported total disagrees with what was actually served.
    #[test]
    fn an_inconsistent_total_makes_the_sweep_partial() {
        let (postings, total) = parse(PAGE_FULL);
        assert_eq!(total, Some(42));
        let (outcome, diagnostic) = assess(postings.len(), postings.len(), total, None);
        assert_eq!(outcome, RunOutcome::Partial);
        assert!(diagnostic.unwrap().contains("reported 42"));
    }

    /// Hazard 3 of 5: a later page is short before the reported total is reached.
    #[test]
    fn a_truncated_later_page_makes_the_sweep_partial() {
        let (postings, total) = parse(PAGE_TRUNCATED);
        assert_eq!(postings.len(), 1);
        assert_eq!(total, Some(42));
        let (outcome, _) = assess(
            postings.len(),
            postings.len(),
            total,
            Some("page 2 returned 1 of 20 records before the reported total was reached"),
        );
        assert_eq!(outcome, RunOutcome::Partial);
    }

    /// Hazard 4 of 5: the same requisition identifier appears on two pages, so the record count overstates the inventory.
    #[test]
    fn a_duplicate_requisition_across_pages_makes_the_sweep_partial() {
        let (page0, _) = parse(PAGE_FULL);
        let (page1, _) = parse(PAGE_DUPLICATE);
        let mut all = page0.clone();
        all.extend(page1);
        let mut unique: Vec<&str> = Vec::new();
        for posting in &all {
            if !unique.contains(&posting.native_id.as_str()) {
                unique.push(&posting.native_id);
            }
        }
        assert_eq!(all.len(), 4);
        assert_eq!(unique.len(), 3, "R-100001 appears on both pages");
        let (outcome, diagnostic) = assess(all.len(), unique.len(), None, None);
        assert_eq!(outcome, RunOutcome::Partial);
        assert!(diagnostic.unwrap().contains("duplicate requisition"));
    }

    /// Hazard 5 of 5: a record with no description stays valid and is flagged, never dropped.
    #[test]
    fn a_record_with_no_description_is_kept_and_flagged_title_only() {
        let (postings, _) = parse(PAGE_NO_DESCRIPTION);
        assert_eq!(postings.len(), 1);
        assert_eq!(postings[0].title, "Electrical Validation Engineer");
        assert!(postings[0].description_html.is_none());
        assert!(postings[0].is_title_only());
        // Every Workday list record is title-only, which must never on its own make the sweep incomplete.
        let (outcome, _) = assess(1, 1, Some(1), None);
        assert_eq!(outcome, RunOutcome::OkComplete);
    }

    #[test]
    fn a_clean_sweep_whose_total_matches_is_complete() {
        let (outcome, diagnostic) = assess(20, 20, Some(20), None);
        assert_eq!(outcome, RunOutcome::OkComplete);
        assert!(outcome.is_complete_sweep());
        assert!(diagnostic.is_none());
    }

    #[test]
    fn a_tenant_that_genuinely_has_no_postings_is_verified_empty_not_an_error() {
        let (outcome, _) = assess(0, 0, Some(0), None);
        assert_eq!(outcome, RunOutcome::OkVerifiedEmpty);
        assert!(outcome.is_complete_sweep());
    }

    #[test]
    fn a_payload_of_the_wrong_shape_is_a_schema_error_not_an_empty_tenant() {
        assert!(parse_page("{}", "t", "wd3").is_err());
        assert!(parse_page("[]", "t", "wd3").is_err());
        assert!(parse_page(r#"{"postings":[]}"#, "t", "wd3").is_err());
    }
}
