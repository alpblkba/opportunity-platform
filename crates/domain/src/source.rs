use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::types::ParseEnumError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Greenhouse,
    Lever,
    Ashby,
    Workday,
    AcademicHtml,
    /// A careers page with no approved adapter. Registering it records a link for human handoff and is not a claim that jobs were discovered.
    Manual,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Greenhouse => "greenhouse",
            Self::Lever => "lever",
            Self::Ashby => "ashby",
            Self::Workday => "workday",
            Self::AcademicHtml => "academic_html",
            Self::Manual => "manual",
        }
    }

    /// Whether the kind's adapter has ever been verified against a live source by this project. Workday is implemented and fixture tested, but its interface requires a POST that the project's external-access rules do not cover, so it starts life as fixture only rather than unverified-but-hopeful.
    pub fn is_fixture_only_until_opt_in(self) -> bool {
        matches!(self, Self::Workday)
    }

    /// Whether an adapter exists for the kind. A kind without one is registrable so the coverage gap stays visible, but it never reports discovered jobs and is never scanned.
    pub fn has_adapter(self) -> bool {
        matches!(
            self,
            Self::Greenhouse | Self::Lever | Self::Ashby | Self::Workday | Self::AcademicHtml
        )
    }
}

impl fmt::Display for SourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SourceKind {
    type Err = ParseEnumError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "greenhouse" => Self::Greenhouse,
            "lever" => Self::Lever,
            "ashby" => Self::Ashby,
            "workday" => Self::Workday,
            "academic_html" => Self::AcademicHtml,
            "manual" => Self::Manual,
            other => return Err(ParseEnumError::new("source kind", other)),
        })
    }
}

/// Typed endpoint configuration. Each variant carries exactly the identifiers its provider documents, so no code path can build a request from a guessed employer name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceEndpoint {
    Greenhouse {
        board_token: String,
    },
    Lever {
        site_id: String,
    },
    Ashby {
        board_slug: String,
    },
    Workday {
        tenant: String,
        instance: String,
        site: String,
    },
    AcademicHtml {
        url: String,
        /// Name of a built-in selector preset, or `None` when `selectors` supplies them directly. Adding a new institute page is therefore configuration, not code.
        #[serde(default)]
        pattern: Option<String>,
        #[serde(default)]
        selectors: Option<Box<AcademicSelectors>>,
    },
    Manual {
        url: String,
    },
}

/// CSS selectors locating the parts of one listing row. Only `row` and `title` are required; every other part is absent when the page does not publish it, and absent stays absent rather than becoming a default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcademicSelectors {
    /// Selects each listing entry. Every other selector is matched inside one of these.
    pub row: String,
    /// Selects the element whose text is the title; its `href` is the link when it is an anchor.
    pub title: String,
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub department: Option<String>,
    #[serde(default)]
    pub employment_type: Option<String>,
    #[serde(default)]
    pub supervisor: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    /// Selects an element whose text states how many results exist in total, for example "Results 1 - 25 of 44". Without one, a listing that paginates can only be reported as incomplete, never as a verified whole.
    #[serde(default)]
    pub total: Option<String>,
    /// Selects anchors leading to further pages of the same listing. Following them is what turns a first page into a complete sweep.
    #[serde(default)]
    pub page_links: Option<String>,
}

impl SourceEndpoint {
    pub fn kind(&self) -> SourceKind {
        match self {
            Self::Greenhouse { .. } => SourceKind::Greenhouse,
            Self::Lever { .. } => SourceKind::Lever,
            Self::Ashby { .. } => SourceKind::Ashby,
            Self::Workday { .. } => SourceKind::Workday,
            Self::AcademicHtml { .. } => SourceKind::AcademicHtml,
            Self::Manual { .. } => SourceKind::Manual,
        }
    }

    /// Reject empty or placeholder identifiers at the trust boundary so an unconfigured template row can never be enabled and then reported as a checked source.
    pub fn validate(&self) -> Result<(), String> {
        let fields: Vec<(&str, &str)> = match self {
            Self::Greenhouse { board_token } => vec![("board_token", board_token)],
            Self::Lever { site_id } => vec![("site_id", site_id)],
            Self::Ashby { board_slug } => vec![("board_slug", board_slug)],
            Self::Workday {
                tenant,
                instance,
                site,
            } => {
                vec![("tenant", tenant), ("instance", instance), ("site", site)]
            }
            Self::AcademicHtml { url, .. } | Self::Manual { url } => vec![("url", url)],
        };
        for (name, value) in fields {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return Err(format!("{name} is empty"));
            }
            // The marker is looked for anywhere in the value, not only at the start, so a template URL that embeds it is caught too.
            if trimmed.to_ascii_uppercase().contains("REPLACE_WITH") {
                return Err(format!(
                    "{name} is still the unconfigured template placeholder"
                ));
            }
            if matches!(
                self,
                Self::Greenhouse { .. } | Self::Lever { .. } | Self::Ashby { .. }
            ) && !trimmed
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err(format!(
                    "{name} must be a board identifier, not a URL or path"
                ));
            }
        }
        Ok(())
    }
}

/// Operational health of a source, derived from its run history rather than asserted by configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceHealth {
    Unknown,
    Healthy,
    Degraded,
    Blocked,
    SchemaError,
}

impl SourceHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Blocked => "blocked",
            Self::SchemaError => "schema_error",
        }
    }

    pub fn from_outcome(outcome: crate::completeness::RunOutcome) -> Self {
        use crate::completeness::RunOutcome as O;
        match outcome {
            O::OkComplete | O::OkVerifiedEmpty | O::NotModified => Self::Healthy,
            O::Partial | O::HttpError | O::RateLimited => Self::Degraded,
            O::AccessDenied => Self::Blocked,
            O::SchemaError => Self::SchemaError,
        }
    }
}

impl fmt::Display for SourceHealth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Honest coverage label shown on the dashboard. This is about what has been proven, not about what the code can in principle do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceSupport {
    /// An adapter exists and a real read against this source succeeded.
    VerifiedLive,
    /// An adapter exists and passes golden fixtures, but this source has never been read successfully.
    FixtureOnly,
    /// An adapter exists but the source refused access.
    Blocked,
    /// No adapter; the registration exists so a human can follow the link.
    ManualLink,
    /// Registered but never attempted.
    NotConfigured,
}

impl SourceSupport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::VerifiedLive => "verified_live",
            Self::FixtureOnly => "fixture_only",
            Self::Blocked => "blocked",
            Self::ManualLink => "manual_link",
            Self::NotConfigured => "not_configured",
        }
    }
}

impl fmt::Display for SourceSupport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_placeholders_cannot_be_enabled_as_real_sources() {
        let endpoint = SourceEndpoint::Greenhouse {
            board_token: "REPLACE_WITH_VERIFIED_BOARD_TOKEN".into(),
        };
        assert!(endpoint.validate().is_err());
    }

    #[test]
    fn empty_identifiers_are_rejected() {
        assert!(SourceEndpoint::Lever {
            site_id: "   ".into()
        }
        .validate()
        .is_err());
        assert!(SourceEndpoint::Workday {
            tenant: "t".into(),
            instance: String::new(),
            site: "s".into()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn a_url_pasted_into_a_board_token_field_is_rejected() {
        let endpoint = SourceEndpoint::Greenhouse {
            board_token: "https://boards.greenhouse.io/example".into(),
        };
        assert!(endpoint.validate().is_err());
    }

    #[test]
    fn a_template_url_with_an_embedded_placeholder_is_rejected() {
        let endpoint = SourceEndpoint::AcademicHtml {
            url: "https://example.org/REPLACE_WITH_VERIFIED_PATH".into(),
            pattern: None,
            selectors: None,
        };
        assert!(endpoint.validate().is_err());
    }

    #[test]
    fn a_plausible_board_identifier_is_accepted() {
        assert!(SourceEndpoint::Greenhouse {
            board_token: "example-board".into()
        }
        .validate()
        .is_ok());
        assert!(SourceEndpoint::Ashby {
            board_slug: "Example_Board".into()
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn a_manual_watch_entry_is_never_claimed_as_a_discovery_source() {
        assert!(!SourceKind::Manual.has_adapter());
        assert!(SourceKind::Greenhouse.has_adapter());
        assert!(SourceKind::Workday.has_adapter());
        assert!(SourceKind::AcademicHtml.has_adapter());
    }
}
