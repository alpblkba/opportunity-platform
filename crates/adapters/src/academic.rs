//! Academic listing adapter.
//!
//! University and institute pages have no common API, so the row and field locations are CSS selectors carried in the source configuration. A new institute is a configuration change rather than a code change, which is the only way this scales past the one page someone wrote an adapter for. Named presets cover layouts that recur across many institutions; anything else supplies its own selectors.
//!
//! This adapter reads the listing page only. It never follows a row through to a job detail or application page, so records carry what the listing itself published and are flagged title-only when that is just a heading.

use opportunity_domain::{
    academic, AcademicSelectors, AdvertisedState, FetchedPosting, RunOutcome, ScanResult,
    SourceRunStats,
};
use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::client::HttpClient;
use crate::location;

/// Selector presets for layouts that recur across many institutions. `successfactors` covers SAP SuccessFactors career sites, which a large number of German universities and research institutes run.
pub fn preset(name: &str) -> Option<AcademicSelectors> {
    match name {
        "successfactors" => Some(AcademicSelectors {
            row: "tr.data-row".into(),
            title: "a.jobTitle-link".into(),
            link: None,
            location: Some("span.jobLocation".into()),
            department: Some("span.jobDepartment".into()),
            employment_type: Some("span.jobFacility".into()),
            supervisor: None,
            body: None,
            total: Some("span.paginationLabel".into()),
            // SuccessFactors renders First, numbered and Last links but no "next", so every numbered anchor is collected and the ones not yet visited are followed.
            page_links: Some("ul.pagination a".into()),
        }),
        _ => None,
    }
}

/// Largest number of listing pages one sweep will fetch. A listing longer than this reports `partial` rather than being silently truncated.
const MAX_PAGES: usize = 10;

/// Pull the total result count out of a counter element's text, for example "Results 1 - 25 of 44" yields 44. The largest number in the text wins, because these counters always state the range before the total.
pub fn parse_total(text: &str) -> Option<i32> {
    text.split(|c: char| !c.is_ascii_digit())
        .filter(|t| !t.is_empty())
        .filter_map(|t| t.parse::<i32>().ok())
        .max()
}

/// What a page of this shape is offering when its own text does not say.
///
/// A careers site lists advertised vacancies by definition, so defaulting those to `unknown` would under-report real jobs. An institute's own topic page is the opposite case, and a page whose shape this tool does not recognise gets no default at all. The text always wins: a `Masterarbeit` row on a careers site is still a topic.
pub fn preset_default_state(name: &str) -> Option<AdvertisedState> {
    match name {
        "successfactors" => Some(AdvertisedState::Advertised),
        _ => None,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AcademicError {
    #[error("no selectors: pattern '{0}' is not a known preset and no explicit selectors were configured")]
    UnknownPattern(String),
    #[error("source has neither a selector preset nor explicit selectors")]
    NoSelectors,
    #[error("selector '{0}' is not valid CSS")]
    BadSelector(String),
    #[error("the page parsed but no row matched '{0}', so the layout has probably changed")]
    NoRows(String),
}

/// Resolve the selector set for a source, preferring explicit selectors over a preset name.
pub fn resolve_selectors(
    pattern: Option<&str>,
    selectors: Option<&AcademicSelectors>,
) -> Result<AcademicSelectors, AcademicError> {
    if let Some(explicit) = selectors {
        return Ok(explicit.clone());
    }
    match pattern {
        Some(name) => preset(name).ok_or_else(|| AcademicError::UnknownPattern(name.to_owned())),
        None => Err(AcademicError::NoSelectors),
    }
}

fn compile(raw: &str) -> Result<Selector, AcademicError> {
    Selector::parse(raw).map_err(|_| AcademicError::BadSelector(raw.to_owned()))
}

fn text_of(row: ElementRef<'_>, raw: Option<&String>) -> Option<String> {
    let selector = compile(raw?).ok()?;
    let value = row.select(&selector).next()?.text().collect::<String>();
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    (!value.is_empty()).then_some(value)
}

/// Parse a listing page. Separated from the request so fixtures and stored snapshots replay offline.
pub fn parse(
    html: &str,
    base_url: &str,
    selectors: &AcademicSelectors,
    default_state: Option<AdvertisedState>,
) -> Result<PageContents, AcademicError> {
    let document = Html::parse_document(html);
    let row_selector = compile(&selectors.row)?;
    let title_selector = compile(&selectors.title)?;
    let link_selector = selectors.link.as_deref().map(compile).transpose()?;
    let base = Url::parse(base_url).ok();

    let mut postings = Vec::new();
    for row in document.select(&row_selector) {
        let Some(title_element) = row.select(&title_selector).next() else {
            continue;
        };
        let title = title_element.text().collect::<String>();
        let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        if title.is_empty() {
            continue;
        }

        let href = match &link_selector {
            Some(selector) => row
                .select(selector)
                .next()
                .and_then(|e| e.value().attr("href")),
            None => title_element.value().attr("href"),
        };
        // A listing whose row carries no link still describes a real topic, so it keeps the page URL rather than being dropped.
        let url = href
            .and_then(|h| base.as_ref().and_then(|b| b.join(h).ok()))
            .map(|u| u.to_string())
            .unwrap_or_else(|| base_url.to_owned());

        let location_raw = text_of(row, selectors.location.as_ref());
        let body = text_of(row, selectors.body.as_ref());
        let row_text = row.text().collect::<Vec<_>>().join("\n");

        let (city, country_from_city) = match location_raw.as_deref().and_then(location::city) {
            Some((city, code)) => (Some(city), Some(code)),
            None => (None, None),
        };

        // The supervisor is taken from a dedicated selector when the page has one, and otherwise only from an explicit label in the row text. It is never inferred from a name appearing nearby.
        let supervisor = text_of(row, selectors.supervisor.as_ref())
            .or_else(|| academic::extract_supervisor(&row_text));

        // The row's own wording decides; the page-shape default only fills the gap when the text says nothing either way.
        let stated = academic::academic_state(&title, body.as_deref().or(Some(&row_text)));
        let state = match (stated, default_state) {
            (AdvertisedState::Unknown, Some(fallback)) => fallback,
            (stated, _) => stated,
        };

        postings.push(FetchedPosting {
            // The link is the only identifier these pages reliably offer; a title alone would collide between two topics of the same name.
            native_id: stable_id(&url, &title),
            url,
            title,
            description_html: body.clone(),
            country_code: location_raw
                .as_deref()
                .and_then(location::country_code)
                .or(country_from_city),
            city,
            remote_raw: location_raw.as_deref().and_then(location::remote_hint),
            department: text_of(row, selectors.department.as_ref()),
            employment_type_raw: text_of(row, selectors.employment_type.as_ref()),
            posted_at: None,
            supervisor,
            contact: None,
            deadline_at: academic::extract_deadline(&row_text),
            advertised_state: Some(state),
            location_raw,
        });
    }

    if postings.is_empty() {
        return Err(AcademicError::NoRows(selectors.row.clone()));
    }

    let total = selectors
        .total
        .as_deref()
        .and_then(|raw| compile(raw).ok())
        .and_then(|sel| document.select(&sel).next())
        .map(|e| e.text().collect::<String>())
        .and_then(|text| parse_total(&text));

    let mut page_urls = Vec::new();
    if let Some(sel) = selectors
        .page_links
        .as_deref()
        .and_then(|raw| compile(raw).ok())
    {
        for anchor in document.select(&sel) {
            if let Some(href) = anchor.value().attr("href") {
                if let Some(resolved) = base.as_ref().and_then(|b| b.join(href).ok()) {
                    let resolved = resolved.to_string();
                    if !page_urls.contains(&resolved) {
                        page_urls.push(resolved);
                    }
                }
            }
        }
    }

    Ok(PageContents {
        postings,
        total,
        page_urls,
    })
}

/// One listing page: the rows it carried, the total it claimed, and the further pages it linked to.
#[derive(Debug, Clone)]
pub struct PageContents {
    pub postings: Vec<FetchedPosting>,
    pub total: Option<i32>,
    pub page_urls: Vec<String>,
}

fn stable_id(url: &str, title: &str) -> String {
    opportunity_domain::content_hash(title, None, None, url)[..24].to_owned()
}

pub async fn scan(
    client: &HttpClient,
    url: &str,
    pattern: Option<&str>,
    selectors: Option<&AcademicSelectors>,
) -> ScanResult {
    let selectors = match resolve_selectors(pattern, selectors) {
        Ok(selectors) => selectors,
        Err(error) => return ScanResult::failed(RunOutcome::SchemaError, error.to_string()),
    };
    let default_state = pattern.and_then(preset_default_state);

    let mut queue: Vec<String> = vec![url.to_owned()];
    let mut visited: Vec<String> = Vec::new();
    let mut postings: Vec<FetchedPosting> = Vec::new();
    let mut total: Option<i32> = None;
    let mut pages_fetched = 0i32;
    let mut first_body = None;
    let mut first_url = None;

    while let Some(next) = queue.first().cloned() {
        queue.remove(0);
        if visited.contains(&next) {
            continue;
        }
        visited.push(next.clone());

        let response = match client.get_text(&next).await {
            Ok(response) => response,
            Err(error) => {
                return partial_or_failure(
                    postings,
                    pages_fetched,
                    total,
                    RunOutcome::HttpError,
                    error.to_string(),
                )
            }
        };
        if let Some(outcome) = crate::status_outcome(response.status) {
            let detail = format!("academic page responded {}", response.status);
            return partial_or_failure(postings, pages_fetched, total, outcome, detail);
        }
        let page = match parse(
            &response.body,
            &response.final_url,
            &selectors,
            default_state,
        ) {
            Ok(page) => page,
            Err(error) => {
                // Zero matching rows on the first page means the layout changed, not that the institute has no openings. On a later page it means the sweep is incomplete, and what was already read is kept.
                return partial_or_failure(
                    postings,
                    pages_fetched,
                    total,
                    RunOutcome::SchemaError,
                    error.to_string(),
                );
            }
        };
        pages_fetched += 1;
        total = page.total.or(total);
        for posting in page.postings {
            // A listing that repeats a row across pages describes one opening twice; counting it twice would make an incomplete sweep look complete.
            if !postings.iter().any(|p| p.native_id == posting.native_id) {
                postings.push(posting);
            }
        }
        if first_body.is_none() {
            first_url = Some(response.final_url);
            first_body = Some(response.body);
        }
        if pages_fetched as usize >= MAX_PAGES {
            break;
        }
        for candidate in page.page_urls {
            if !visited.contains(&candidate) && !queue.contains(&candidate) {
                queue.push(candidate);
            }
        }
    }

    let stats = SourceRunStats {
        pages_fetched,
        records_seen: postings.len() as i32,
        total_reported: total,
    };

    // A sweep is only complete when the listing's own total was reached. Without a stated total there is no way to know a paginated listing was fully read, so a page cap or an unfollowed link leaves it partial.
    let (outcome, diagnostic) = match total {
        Some(stated) if (postings.len() as i32) < stated => (
            RunOutcome::Partial,
            Some(format!(
                "listing states {stated} results but the sweep read {} after {pages_fetched} pages",
                postings.len()
            )),
        ),
        Some(_) => (RunOutcome::OkComplete, None),
        None if !queue.is_empty() || pages_fetched as usize >= MAX_PAGES => (
            RunOutcome::Partial,
            Some(format!(
                "listing paginates but states no total, so {pages_fetched} pages cannot be confirmed as the whole listing"
            )),
        ),
        None => (RunOutcome::OkComplete, None),
    };

    ScanResult {
        outcome,
        postings,
        stats,
        diagnostic,
        raw_url: first_url,
        raw_body: first_body,
    }
}

/// A failure after some pages already succeeded keeps those records and reports `partial`, because what was read is real and what was not is unknown rather than gone.
fn partial_or_failure(
    postings: Vec<FetchedPosting>,
    pages_fetched: i32,
    total: Option<i32>,
    outcome: RunOutcome,
    diagnostic: String,
) -> ScanResult {
    if postings.is_empty() {
        return ScanResult::failed(outcome, diagnostic);
    }
    let stats = SourceRunStats {
        pages_fetched,
        records_seen: postings.len() as i32,
        total_reported: total,
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

    const THESIS_PAGE: &str = include_str!("../tests/fixtures/academic-thesis-list.html");
    const SUCCESSFACTORS_PAGE: &str =
        include_str!("../tests/fixtures/academic-successfactors.html");
    const PAGINATED_PAGE: &str =
        include_str!("../tests/fixtures/academic-successfactors-paginated.html");

    fn thesis_selectors() -> AcademicSelectors {
        AcademicSelectors {
            row: "article.thesis".into(),
            title: "h3 a".into(),
            link: None,
            location: Some(".ort".into()),
            department: Some(".institut".into()),
            employment_type: Some(".art".into()),
            supervisor: Some(".betreuer".into()),
            body: Some(".beschreibung".into()),
            total: None,
            page_links: None,
        }
    }

    fn parse_thesis() -> Vec<FetchedPosting> {
        parse(
            THESIS_PAGE,
            "https://example-institut.test/abschlussarbeiten",
            &thesis_selectors(),
            None,
        )
        .unwrap()
        .postings
    }

    #[test]
    fn a_german_thesis_listing_parses_with_its_supervisor_and_deadline() {
        let postings = parse_thesis();
        assert_eq!(postings.len(), 4);
        let thesis = &postings[0];
        assert_eq!(
            thesis.title,
            "Masterarbeit: Seitenmigration in heterogenen Speichersystemen"
        );
        assert_eq!(
            thesis.supervisor.as_deref(),
            Some("Prof. Dr. Anna Beispiel")
        );
        assert_eq!(thesis.city.as_deref(), Some("Karlsruhe"));
        assert_eq!(thesis.country_code.as_deref(), Some("DE"));
        assert_eq!(
            thesis.url,
            "https://example-institut.test/arbeiten/seitenmigration"
        );
        let deadline = thesis.deadline_at.expect("this topic states a deadline");
        assert_eq!(deadline.format("%Y-%m-%d").to_string(), "2026-12-31");
    }

    #[test]
    fn an_open_topic_is_never_represented_as_an_advertised_funded_vacancy() {
        let postings = parse_thesis();
        assert_eq!(
            postings[0].advertised_state,
            Some(AdvertisedState::OpenTopic)
        );
        let funded = postings
            .iter()
            .find(|p| p.title.contains("Wissenschaftliche"))
            .unwrap();
        assert_eq!(
            funded.advertised_state,
            Some(AdvertisedState::Advertised),
            "a post stating TV-L pay is a real vacancy"
        );
    }

    #[test]
    fn a_supervisor_page_is_marked_potential_supervisor_and_implies_no_availability() {
        let postings = parse_thesis();
        let page = postings
            .iter()
            .find(|p| p.title.contains("Forschungsinteressen"))
            .unwrap();
        assert_eq!(
            page.advertised_state,
            Some(AdvertisedState::PotentialSupervisor)
        );
        assert!(page.deadline_at.is_none());
    }

    #[test]
    fn a_topic_without_a_stated_deadline_keeps_an_absent_deadline() {
        let postings = parse_thesis();
        let rolling = postings
            .iter()
            .find(|p| p.title.contains("RISC-V"))
            .unwrap();
        assert_eq!(rolling.advertised_state, Some(AdvertisedState::OpenTopic));
        assert!(
            rolling.deadline_at.is_none(),
            "no stated deadline must stay absent, not become a default"
        );
        assert!(rolling.supervisor.is_some());
    }

    #[test]
    fn the_successfactors_preset_parses_a_real_world_career_site_layout() {
        let selectors = preset("successfactors").unwrap();
        let postings = parse(
            SUCCESSFACTORS_PAGE,
            "https://jobs.example.test/search/",
            &selectors,
            preset_default_state("successfactors"),
        )
        .unwrap()
        .postings;
        assert_eq!(postings.len(), 3);
        assert_eq!(postings[0].title, "PhD position (m/f/x) - Memory Systems");
        assert_eq!(
            postings[0].department.as_deref(),
            Some("Fachgebiet: Rechnerarchitektur")
        );
        assert_eq!(postings[0].employment_type_raw.as_deref(), Some("Vollzeit"));
        assert_eq!(
            postings[0].url,
            "https://jobs.example.test/EX/job/Potsdam-PhD-position-Memory-Systems/1234/"
        );
        assert_eq!(postings[0].country_code.as_deref(), Some("DE"));
    }

    #[test]
    fn a_careers_site_lists_vacancies_by_default_but_a_thesis_row_still_reads_as_a_topic() {
        let selectors = preset("successfactors").unwrap();
        let postings = parse(
            SUCCESSFACTORS_PAGE,
            "https://jobs.example.test/search/",
            &selectors,
            preset_default_state("successfactors"),
        )
        .unwrap()
        .postings;
        assert_eq!(
            postings[0].advertised_state,
            Some(AdvertisedState::Advertised),
            "a PhD position on a careers site is an advertised vacancy, not an unknown"
        );
        // Without a page-shape default, the same rows stay unknown rather than being guessed.
        let undecided = parse(
            SUCCESSFACTORS_PAGE,
            "https://jobs.example.test/search/",
            &selectors,
            None,
        )
        .unwrap()
        .postings;
        assert_eq!(
            undecided[0].advertised_state,
            Some(AdvertisedState::Unknown)
        );
    }

    #[test]
    fn a_thesis_row_on_a_careers_site_overrides_the_advertised_default() {
        let selectors = thesis_selectors();
        let postings = parse(
            THESIS_PAGE,
            "https://example-institut.test/abschlussarbeiten",
            &selectors,
            Some(AdvertisedState::Advertised),
        )
        .unwrap()
        .postings;
        assert_eq!(
            postings[0].advertised_state,
            Some(AdvertisedState::OpenTopic),
            "the row's own wording must beat the page-shape default"
        );
    }

    #[test]
    fn a_stated_total_is_read_from_the_counter_text() {
        assert_eq!(parse_total("Results 1 – 25 of 44"), Some(44));
        assert_eq!(parse_total("Ergebnisse 1 - 10 von 7"), Some(10));
        assert_eq!(parse_total("no numbers here"), None);
    }

    #[test]
    fn a_first_page_of_a_paginated_listing_reports_its_total_and_its_further_pages() {
        let selectors = preset("successfactors").unwrap();
        let page = parse(
            PAGINATED_PAGE,
            "https://jobs.example.test/search/",
            &selectors,
            preset_default_state("successfactors"),
        )
        .unwrap();
        assert_eq!(page.postings.len(), 2);
        assert_eq!(
            page.total,
            Some(44),
            "the counter states more results than this page carries"
        );
        assert!(
            page.page_urls.iter().any(|u| u.contains("startrow=25")),
            "the second page must be discovered from the pagination links: {:?}",
            page.page_urls
        );
    }

    #[test]
    fn a_listing_without_a_pagination_counter_has_no_total_to_check_against() {
        let selectors = preset("successfactors").unwrap();
        let page = parse(
            SUCCESSFACTORS_PAGE,
            "https://jobs.example.test/search/",
            &selectors,
            None,
        )
        .unwrap();
        assert_eq!(page.total, None);
        assert!(page.page_urls.is_empty());
    }

    #[test]
    fn a_layout_change_is_a_schema_error_rather_than_an_empty_institute() {
        let selectors = preset("successfactors").unwrap();
        let error = parse(THESIS_PAGE, "https://example.test/", &selectors, None).unwrap_err();
        assert!(matches!(error, AcademicError::NoRows(_)), "got {error}");
    }

    #[test]
    fn an_unknown_preset_is_refused_rather_than_silently_matching_nothing() {
        assert!(matches!(
            resolve_selectors(Some("not-a-preset"), None),
            Err(AcademicError::UnknownPattern(_))
        ));
        assert!(matches!(
            resolve_selectors(None, None),
            Err(AcademicError::NoSelectors)
        ));
        assert!(resolve_selectors(Some("successfactors"), None).is_ok());
    }

    #[test]
    fn explicit_selectors_win_over_a_preset_name_so_a_new_institute_needs_no_code() {
        let custom = thesis_selectors();
        let resolved = resolve_selectors(Some("successfactors"), Some(&custom)).unwrap();
        assert_eq!(resolved.row, "article.thesis");
    }

    #[test]
    fn two_topics_sharing_a_title_do_not_collide_on_one_identifier() {
        let postings = parse_thesis();
        let mut ids: Vec<&str> = postings.iter().map(|p| p.native_id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), postings.len());
    }
}
