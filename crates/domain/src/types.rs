use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::completeness::RunOutcome;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpportunityKind {
    Employment,
    PartTime,
    Internship,
    WorkingStudent,
    MasterThesis,
    ResearchAssistant,
    Phd,
    Fellowship,
    Other,
}

impl OpportunityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Employment => "employment",
            Self::PartTime => "part_time",
            Self::Internship => "internship",
            Self::WorkingStudent => "working_student",
            Self::MasterThesis => "master_thesis",
            Self::ResearchAssistant => "research_assistant",
            Self::Phd => "phd",
            Self::Fellowship => "fellowship",
            Self::Other => "other",
        }
    }
}

impl fmt::Display for OpportunityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OpportunityKind {
    type Err = ParseEnumError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "employment" => Self::Employment,
            "part_time" => Self::PartTime,
            "internship" => Self::Internship,
            "working_student" => Self::WorkingStudent,
            "master_thesis" => Self::MasterThesis,
            "research_assistant" => Self::ResearchAssistant,
            "phd" => Self::Phd,
            "fellowship" => Self::Fellowship,
            "other" => Self::Other,
            other => return Err(ParseEnumError::new("opportunity kind", other)),
        })
    }
}

/// Whether a record is a real advertised opening, only a topic offer, or merely a contactable supervisor. Conflating these would present a professor's research-interests page as a funded vacancy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvertisedState {
    Advertised,
    OpenTopic,
    PotentialSupervisor,
    Unknown,
}

impl AdvertisedState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Advertised => "advertised",
            Self::OpenTopic => "open_topic",
            Self::PotentialSupervisor => "potential_supervisor",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for AdvertisedState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AdvertisedState {
    type Err = ParseEnumError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "advertised" => Self::Advertised,
            "open_topic" => Self::OpenTopic,
            "potential_supervisor" => Self::PotentialSupervisor,
            "unknown" => Self::Unknown,
            other => return Err(ParseEnumError::new("advertised state", other)),
        })
    }
}

/// Remote eligibility as the source stated it. `Unknown` is the default and is never rewritten into `OnSite`: a source that is silent about work mode has not said the role is on site. The physical work location is stored separately and is not the same question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteEligibility {
    OnSite,
    Hybrid,
    /// Remote, but the source restricted it to a country or region; the restriction text is kept alongside.
    RemoteRestricted,
    RemoteAnywhere,
    Unknown,
}

impl RemoteEligibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OnSite => "on_site",
            Self::Hybrid => "hybrid",
            Self::RemoteRestricted => "remote_restricted",
            Self::RemoteAnywhere => "remote_anywhere",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for RemoteEligibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RemoteEligibility {
    type Err = ParseEnumError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "on_site" => Self::OnSite,
            "hybrid" => Self::Hybrid,
            "remote_restricted" => Self::RemoteRestricted,
            "remote_anywhere" => Self::RemoteAnywhere,
            "unknown" => Self::Unknown,
            other => return Err(ParseEnumError::new("remote eligibility", other)),
        })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown {kind}: {value}")]
pub struct ParseEnumError {
    kind: &'static str,
    value: String,
}

impl ParseEnumError {
    pub(crate) fn new(kind: &'static str, value: &str) -> Self {
        Self {
            kind,
            value: value.to_owned(),
        }
    }
}

/// One posting exactly as an adapter read it, before any persistence decision. Every optional field is `None` when the source did not state it; adapters never substitute a default for a missing value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchedPosting {
    pub native_id: String,
    pub url: String,
    pub title: String,
    pub description_html: Option<String>,
    pub location_raw: Option<String>,
    pub country_code: Option<String>,
    pub city: Option<String>,
    pub remote_raw: Option<String>,
    pub department: Option<String>,
    pub employment_type_raw: Option<String>,
    pub posted_at: Option<DateTime<Utc>>,
    /// Named academic supervisor or contact person, when the source published one. Never inferred.
    pub supervisor: Option<String>,
    /// Contact address or page the source published for enquiries.
    pub contact: Option<String>,
    /// Application deadline the source stated. Absent means the source did not state one, which is not the same as no deadline existing.
    pub deadline_at: Option<DateTime<Utc>>,
    /// What the source is actually offering. `None` lets the caller apply its own default, which for an ATS board is `advertised`.
    pub advertised_state: Option<AdvertisedState>,
}

impl FetchedPosting {
    /// A posting whose source gave no usable description body. Ranking must still accept it, flagged so that a later low lexical score is read as missing text rather than a poor match.
    pub fn is_title_only(&self) -> bool {
        self.description_html
            .as_deref()
            .map(|d| crate::normalize::strip_html(d).chars().count() < 40)
            .unwrap_or(true)
    }
}

/// What one sweep of one source produced. `outcome` carries the completeness verdict and `diagnostic` carries a human-readable reason that must never contain credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanResult {
    pub outcome: RunOutcome,
    pub postings: Vec<FetchedPosting>,
    pub stats: SourceRunStats,
    pub diagnostic: Option<String>,
    /// Raw payload kept for offline reparse. Absent when the source refused or the transport failed.
    pub raw_body: Option<String>,
    pub raw_url: Option<String>,
}

impl ScanResult {
    pub fn failed(outcome: RunOutcome, diagnostic: impl Into<String>) -> Self {
        Self {
            outcome,
            postings: Vec::new(),
            stats: SourceRunStats::default(),
            diagnostic: Some(diagnostic.into()),
            raw_body: None,
            raw_url: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRunStats {
    pub pages_fetched: i32,
    pub records_seen: i32,
    /// Total the source claimed to hold, when it reports one. A mismatch against `records_seen` is what turns an otherwise clean sweep into `Partial`.
    pub total_reported: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn posting(description: Option<&str>) -> FetchedPosting {
        FetchedPosting {
            native_id: "1".into(),
            url: "https://example.org/1".into(),
            title: "Firmware Engineer".into(),
            description_html: description.map(str::to_owned),
            location_raw: None,
            country_code: None,
            city: None,
            remote_raw: None,
            department: None,
            employment_type_raw: None,
            posted_at: None,
            supervisor: None,
            contact: None,
            deadline_at: None,
            advertised_state: None,
        }
    }

    #[test]
    fn a_posting_without_a_description_is_valid_and_flagged_title_only() {
        assert!(posting(None).is_title_only());
        assert!(posting(Some("<p>   </p>")).is_title_only());
        assert!(!posting(Some(
            "<p>Work on memory controller firmware for RISC-V platforms.</p>"
        ))
        .is_title_only());
    }

    #[test]
    fn enums_round_trip_through_their_database_representation() {
        assert_eq!(
            "master_thesis".parse::<OpportunityKind>().unwrap(),
            OpportunityKind::MasterThesis
        );
        assert_eq!(
            "open_topic".parse::<AdvertisedState>().unwrap(),
            AdvertisedState::OpenTopic
        );
        assert_eq!(
            "remote_restricted".parse::<RemoteEligibility>().unwrap(),
            RemoteEligibility::RemoteRestricted
        );
        assert!("nonsense".parse::<OpportunityKind>().is_err());
    }
}
