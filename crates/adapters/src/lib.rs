//! Read-only adapters for public job listings. Requests are unauthenticated reads of a board's documented listing endpoint or of an owner-registered listing page; no adapter here opens an employer application form.

pub mod academic;
pub mod ashby;
pub mod client;
pub mod greenhouse;
pub mod lever;
pub mod location;
pub mod net;
pub mod workday;

use opportunity_domain::{RunOutcome, ScanResult, SourceEndpoint};

pub use client::{HttpClient, HttpError};

/// Dispatch a sweep to the adapter for the configured endpoint. A kind with no adapter returns a refusal with a diagnostic rather than a silent empty result, because an empty result would be indistinguishable from a source that genuinely has no openings.
pub async fn scan(client: &HttpClient, endpoint: &SourceEndpoint) -> ScanResult {
    match endpoint {
        SourceEndpoint::Greenhouse { board_token } => greenhouse::scan(client, board_token).await,
        SourceEndpoint::Lever { site_id } => lever::scan(client, site_id).await,
        SourceEndpoint::Ashby { board_slug } => ashby::scan(client, board_slug).await,
        SourceEndpoint::Workday {
            tenant,
            instance,
            site,
        } => workday::scan(client, tenant, instance, site).await,
        SourceEndpoint::AcademicHtml {
            url,
            pattern,
            selectors,
        } => academic::scan(client, url, pattern.as_deref(), selectors.as_deref()).await,
        SourceEndpoint::Manual { .. } => {
            unsupported("manual watch entry: no adapter, follow the link by hand")
        }
    }
}

fn unsupported(reason: &str) -> ScanResult {
    ScanResult::failed(RunOutcome::AccessDenied, reason)
}

/// Map an HTTP status to a failure outcome, or `None` when the response is usable. The distinction between `access_denied` and `http_error` matters because only the former means the source is refusing this tool rather than having a bad day.
pub(crate) fn status_outcome(status: reqwest::StatusCode) -> Option<RunOutcome> {
    use reqwest::StatusCode;
    match status {
        s if s.is_success() => None,
        StatusCode::NOT_MODIFIED => Some(RunOutcome::NotModified),
        StatusCode::TOO_MANY_REQUESTS => Some(RunOutcome::RateLimited),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => {
            Some(RunOutcome::AccessDenied)
        }
        _ => Some(RunOutcome::HttpError),
    }
}

/// A clean sweep that found nothing is `ok_verified_empty`, which is a positive statement that the board is empty. An incomplete sweep is `partial` whether or not it found anything, so it can never be read as evidence of absence.
pub(crate) fn outcome_for(complete: bool, records: usize) -> RunOutcome {
    match (complete, records) {
        (false, _) => RunOutcome::Partial,
        (true, 0) => RunOutcome::OkVerifiedEmpty,
        (true, _) => RunOutcome::OkComplete,
    }
}
