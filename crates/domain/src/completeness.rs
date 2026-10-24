use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Normalized outcome of one source sweep. The distinction that matters operationally is whether the sweep observed the source's entire current inventory, because only then is an unseen posting evidence of disappearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    /// Every page was fetched and parsed and the source returned at least one record.
    OkComplete,
    /// Every page was fetched and parsed and the source genuinely advertises no openings.
    OkVerifiedEmpty,
    /// Some records were retrieved but the sweep is known to be incomplete: truncated pagination, a blank middle page, or a reported total that does not match what was read.
    Partial,
    /// The source reported no change since the last fetch.
    NotModified,
    /// The source refused access: 401, 403, or an explicit block page.
    AccessDenied,
    /// The response was retrieved but did not match the expected schema.
    SchemaError,
    /// Transport or non-auth HTTP failure.
    HttpError,
    /// The source asked us to slow down: 429 or an explicit Retry-After.
    RateLimited,
}

impl RunOutcome {
    /// True only when the sweep saw the source's whole inventory. A posting absent from such a sweep is evidence of disappearance; absence from any other outcome is not.
    pub fn is_complete_sweep(self) -> bool {
        matches!(self, Self::OkComplete | Self::OkVerifiedEmpty)
    }

    /// True when the sweep produced records worth persisting. Partial sweeps count: the records they did observe are real.
    pub fn has_usable_records(self) -> bool {
        matches!(self, Self::OkComplete | Self::Partial)
    }

    /// True when the source itself is healthy, whether or not it had jobs.
    pub fn is_healthy(self) -> bool {
        matches!(
            self,
            Self::OkComplete | Self::OkVerifiedEmpty | Self::NotModified
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::OkComplete => "ok_complete",
            Self::OkVerifiedEmpty => "ok_verified_empty",
            Self::Partial => "partial",
            Self::NotModified => "not_modified",
            Self::AccessDenied => "access_denied",
            Self::SchemaError => "schema_error",
            Self::HttpError => "http_error",
            Self::RateLimited => "rate_limited",
        }
    }
}

impl fmt::Display for RunOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown run outcome: {0}")]
pub struct UnknownOutcome(String);

impl FromStr for RunOutcome {
    type Err = UnknownOutcome;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "ok_complete" => Self::OkComplete,
            "ok_verified_empty" => Self::OkVerifiedEmpty,
            "partial" => Self::Partial,
            "not_modified" => Self::NotModified,
            "access_denied" => Self::AccessDenied,
            "schema_error" => Self::SchemaError,
            "http_error" => Self::HttpError,
            "rate_limited" => Self::RateLimited,
            other => return Err(UnknownOutcome(other.to_owned())),
        })
    }
}

/// Number of consecutive complete sweeps that must miss a posting before it is treated as closed. Two sweeps absorb a single source-side glitch that still reported itself as complete.
pub const MISSING_SWEEPS_BEFORE_CLOSE: i32 = 2;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_complete_sweeps_can_close_postings() {
        let closers: Vec<RunOutcome> = [
            RunOutcome::OkComplete,
            RunOutcome::OkVerifiedEmpty,
            RunOutcome::Partial,
            RunOutcome::NotModified,
            RunOutcome::AccessDenied,
            RunOutcome::SchemaError,
            RunOutcome::HttpError,
            RunOutcome::RateLimited,
        ]
        .into_iter()
        .filter(|o| o.is_complete_sweep())
        .collect();
        assert_eq!(
            closers,
            vec![RunOutcome::OkComplete, RunOutcome::OkVerifiedEmpty]
        );
    }

    #[test]
    fn partial_sweep_keeps_its_records_but_cannot_close() {
        assert!(RunOutcome::Partial.has_usable_records());
        assert!(!RunOutcome::Partial.is_complete_sweep());
    }

    #[test]
    fn outcome_round_trips_through_its_database_representation() {
        for outcome in [
            RunOutcome::OkComplete,
            RunOutcome::OkVerifiedEmpty,
            RunOutcome::Partial,
            RunOutcome::NotModified,
            RunOutcome::AccessDenied,
            RunOutcome::SchemaError,
            RunOutcome::HttpError,
            RunOutcome::RateLimited,
        ] {
            assert_eq!(outcome.as_str().parse::<RunOutcome>().unwrap(), outcome);
        }
    }
}
