//! What the owner is looking for. A profile is user-owned private data: nothing here is seeded with a real person's preferences, and the shipped example file contains only neutral technical defaults.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::types::ParseEnumError;

/// How strictly an advertised experience level is treated.
///
/// A posting asking for five years is a preference of the employer, not a law. Treating every such line as a hard gate would hide most roles a strong candidate should still see, so the default is to score it rather than exclude on it, and the owner can tighten that per profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeniorityPolicy {
    /// An advertised level only moves the score.
    Preference,
    /// A posting clearly above the configured ceiling is excluded.
    Hard,
    /// Advertised levels are ignored entirely.
    Ignore,
}

impl SeniorityPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::Hard => "hard",
            Self::Ignore => "ignore",
        }
    }
}

impl fmt::Display for SeniorityPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SeniorityPolicy {
    type Err = ParseEnumError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "preference" => Self::Preference,
            "hard" => Self::Hard,
            "ignore" => Self::Ignore,
            other => return Err(ParseEnumError::new("seniority policy", other)),
        })
    }
}

/// Which work arrangements the owner will accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemotePolicy {
    /// On-site, hybrid and remote are all acceptable, subject to the location rules.
    Any,
    /// Only roles that can actually be worked remotely from the current country.
    RemoteFromCurrentCountryOnly,
    /// Only roles with a physical location in a preferred country, or reachable by relocation.
    OnSiteOrRelocate,
}

impl RemotePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::RemoteFromCurrentCountryOnly => "remote_from_current_country_only",
            Self::OnSiteOrRelocate => "on_site_or_relocate",
        }
    }
}

impl fmt::Display for RemotePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RemotePolicy {
    type Err = ParseEnumError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "any" => Self::Any,
            "remote_from_current_country_only" => Self::RemoteFromCurrentCountryOnly,
            "on_site_or_relocate" => Self::OnSiteOrRelocate,
            other => return Err(ParseEnumError::new("remote policy", other)),
        })
    }
}

/// Relative weight of each scoring factor. Exposed so the owner can see and change why one profile ranks differently from another.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Weights {
    pub terms: f32,
    pub concepts: f32,
    pub title: f32,
    pub location: f32,
    pub kind: f32,
    pub technology: f32,
    pub freshness: f32,
    pub semantic: f32,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            terms: 1.0,
            concepts: 1.0,
            // A match in the title is stronger evidence than the same word buried in boilerplate.
            title: 2.0,
            location: 1.0,
            kind: 1.5,
            technology: 1.0,
            freshness: 0.5,
            semantic: 2.0,
        }
    }
}

/// One saved search over the corpus, with everything needed to decide eligibility and rank what remains.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub key: String,
    pub display_name: String,
    /// Ordering when several profiles are shown together. Lower sorts first.
    pub priority: i32,
    pub enabled: bool,

    /// Opportunity kinds this profile is interested in. Empty means any kind.
    pub job_kinds: Vec<String>,
    /// Free-text interests. Each is matched literally and, when it belongs to a known concept, through that concept's other surface forms.
    pub terms: Vec<String>,
    /// Terms that count against a match without excluding it. Hard exclusions live in the rules table, not here.
    pub negative_terms: Vec<String>,
    pub preferred_technologies: Vec<String>,
    pub excluded_technologies: Vec<String>,

    pub preferred_countries: Vec<String>,
    pub preferred_cities: Vec<String>,
    /// Where the owner is now. Only used to decide whether a region-locked remote role is workable, never published anywhere.
    pub current_country: Option<String>,
    pub relocation_willing: bool,
    pub remote_policy: RemotePolicy,
    /// Keep opportunities whose location a parser could not determine. On by default, because discarding them would hide real roles behind a parsing gap.
    pub include_unknown_location: bool,

    pub min_years_experience: Option<i32>,
    pub max_years_experience: Option<i32>,
    pub seniority_policy: SeniorityPolicy,
    /// Highest degree held, used to flag a posting that asks for more. Never used to invent a qualification.
    pub education_level: Option<String>,
    /// Countries the owner is already authorized to work in. Used only to mark an unanswered authorization question for review.
    pub work_authorization: Vec<String>,

    pub weights: Weights,
}

impl Profile {
    /// A neutral starting profile. Deliberately empty of personal detail: the owner fills it in through the interface, and nothing here is derived from any real person.
    pub fn blank(key: &str, display_name: &str) -> Self {
        Self {
            key: key.to_owned(),
            display_name: display_name.to_owned(),
            priority: 100,
            enabled: true,
            job_kinds: Vec::new(),
            terms: Vec::new(),
            negative_terms: Vec::new(),
            preferred_technologies: Vec::new(),
            excluded_technologies: Vec::new(),
            preferred_countries: Vec::new(),
            preferred_cities: Vec::new(),
            current_country: None,
            relocation_willing: false,
            remote_policy: RemotePolicy::Any,
            include_unknown_location: true,
            min_years_experience: None,
            max_years_experience: None,
            seniority_policy: SeniorityPolicy::Preference,
            education_level: None,
            work_authorization: Vec::new(),
            weights: Weights::default(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.key.trim().is_empty() {
            return Err("key is empty".into());
        }
        if self.display_name.trim().is_empty() {
            return Err("display name is empty".into());
        }
        for code in self
            .preferred_countries
            .iter()
            .chain(self.current_country.iter())
        {
            if code.len() != 2 || !code.chars().all(|c| c.is_ascii_uppercase()) {
                return Err(format!(
                    "'{code}' is not a two-letter uppercase country code"
                ));
            }
        }
        if let (Some(min), Some(max)) = (self.min_years_experience, self.max_years_experience) {
            if min > max {
                return Err("minimum years of experience exceeds the maximum".into());
            }
        }
        if self
            .min_years_experience
            .is_some_and(|y| !(0..=60).contains(&y))
            || self
                .max_years_experience
                .is_some_and(|y| !(0..=60).contains(&y))
        {
            return Err("years of experience must be between 0 and 60".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_profile_carries_no_personal_detail_and_hides_nothing_by_default() {
        let profile = Profile::blank("example", "Example");
        assert!(profile.terms.is_empty());
        assert!(profile.preferred_countries.is_empty());
        assert!(profile.current_country.is_none());
        assert!(profile.work_authorization.is_empty());
        assert!(
            profile.include_unknown_location,
            "a parsing gap must not hide real roles"
        );
        assert_eq!(profile.seniority_policy, SeniorityPolicy::Preference);
        assert_eq!(profile.remote_policy, RemotePolicy::Any);
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn an_invalid_country_code_or_experience_range_is_rejected() {
        let mut profile = Profile::blank("example", "Example");
        profile.preferred_countries = vec!["Germany".into()];
        assert!(profile.validate().is_err());
        profile.preferred_countries = vec!["de".into()];
        assert!(profile.validate().is_err());
        profile.preferred_countries = vec!["DE".into()];
        assert!(profile.validate().is_ok());

        profile.min_years_experience = Some(8);
        profile.max_years_experience = Some(3);
        assert!(profile.validate().is_err());
        profile.max_years_experience = Some(12);
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn enums_round_trip_through_their_database_representation() {
        for policy in [
            SeniorityPolicy::Preference,
            SeniorityPolicy::Hard,
            SeniorityPolicy::Ignore,
        ] {
            assert_eq!(policy.as_str().parse::<SeniorityPolicy>().unwrap(), policy);
        }
        for policy in [
            RemotePolicy::Any,
            RemotePolicy::RemoteFromCurrentCountryOnly,
            RemotePolicy::OnSiteOrRelocate,
        ] {
            assert_eq!(policy.as_str().parse::<RemotePolicy>().unwrap(), policy);
        }
    }
}
