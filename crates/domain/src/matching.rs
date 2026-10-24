//! Deciding whether an opportunity fits a profile, and saying why.
//!
//! Two different questions are kept apart on purpose. Eligibility is a hard yes or no with a stated reason: an excluded employer, a banned keyword, a location that cannot work. Fit is a score made of named factors, each carrying the evidence that produced it. Mixing them would turn a preference into a gate and hide real roles.
//!
//! Nothing here invents a fact. A posting that does not state its seniority, location, salary or sponsorship produces an uncertainty, never an assumption.

use serde::{Deserialize, Serialize};

use crate::concepts;
use crate::normalize::{contains_phrase, normalize_location, normalize_organization};
use crate::profile::{Profile, RemotePolicy, SeniorityPolicy};

/// What a scan knows about one opportunity when it is scored. Built by the caller from stored rows; this module performs no I/O.
#[derive(Debug, Clone, Default)]
pub struct Candidate {
    pub title: String,
    pub organization: String,
    pub description: Option<String>,
    pub department: Option<String>,
    pub kind: String,
    pub advertised_state: String,
    pub country_code: Option<String>,
    pub city: Option<String>,
    pub remote_raw: Option<String>,
    /// Days since the opportunity was first seen, used only as a mild freshness factor.
    pub age_days: Option<i64>,
    pub title_only: bool,
}

/// A hard rule configured by the owner. Company rules match a normalized exact alias so that `Core Systems` never blocks `Hardcore Systems`; term rules match whole phrases in the scoped field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExclusionRule {
    pub kind: ExclusionKind,
    pub value: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExclusionKind {
    /// Normalized exact employer identity, including aliases.
    Company,
    /// Phrase in the job title.
    TitleTerm,
    /// Phrase in the description body.
    DescriptionTerm,
    /// A single opportunity the owner hid by hand.
    Hidden,
}

impl ExclusionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Company => "company",
            Self::TitleTerm => "title_term",
            Self::DescriptionTerm => "description_term",
            Self::Hidden => "hidden",
        }
    }
}

/// Why an opportunity was excluded, in words the owner can act on, with the text that triggered it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exclusion {
    pub reason: String,
    pub evidence: Option<String>,
    /// True when the owner can lift this by editing a rule, as opposed to a structural mismatch.
    pub reversible: bool,
}

/// Something the posting did not say. Surfaced rather than guessed, and carried into the application workflow in later sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Uncertainty {
    pub field: String,
    pub detail: String,
}

/// One named contribution to the score, with the evidence behind it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Factor {
    pub name: String,
    pub contribution: f32,
    pub detail: String,
    pub evidence: Vec<String>,
}

/// The complete verdict for one opportunity against one profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchResult {
    /// 0 to 100 after normalization. Only meaningful for ranking against other opportunities scored by the same profile and model version.
    pub score: f32,
    pub eligible: bool,
    pub exclusions: Vec<Exclusion>,
    pub factors: Vec<Factor>,
    pub uncertainties: Vec<Uncertainty>,
    pub matched_concepts: Vec<String>,
    /// Names the scoring method honestly. Never says AI unless a model actually ran.
    pub model: String,
}

/// Identifies the deterministic scorer in stored results and in the interface, so a score can always be traced to the rules that produced it.
pub const LEXICAL_MODEL: &str = "lexical-concepts-v1";

/// How a remote arrangement was stated by the source, as read from its own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteScope {
    /// Remote with no stated geographic limit.
    Anywhere,
    /// Remote, but the source limited it to these country codes.
    RestrictedTo(Vec<String>),
    Hybrid,
    /// The source said nothing about remote work, which is not the same as saying it is on site.
    Unstated,
}

/// Read the source's own remote wording. A restriction is only reported when the text actually names a place.
pub fn remote_scope(remote_raw: Option<&str>, country_code: Option<&str>) -> RemoteScope {
    let Some(raw) = remote_raw else {
        return RemoteScope::Unstated;
    };
    let folded = normalize_location(Some(raw));
    if folded.split(' ').any(|t| t == "hybrid") {
        return RemoteScope::Hybrid;
    }
    if !folded
        .split(' ')
        .any(|t| matches!(t, "remote" | "anywhere" | "distributed"))
    {
        return RemoteScope::Unstated;
    }
    // Wording such as "Remote - US only" or "Remote (Germany)" limits where the holder may live. The country the source attached to the posting is the restriction when the text itself only says "only".
    let mut restricted: Vec<String> = Vec::new();
    for (name, code) in COUNTRY_WORDS {
        if folded.split(' ').any(|t| t == *name) {
            restricted.push((*code).to_owned());
        }
    }
    if restricted.is_empty() && folded.split(' ').any(|t| t == "only") {
        if let Some(code) = country_code {
            restricted.push(code.to_owned());
        }
    }
    if restricted.is_empty() {
        RemoteScope::Anywhere
    } else {
        RemoteScope::RestrictedTo(restricted)
    }
}

/// Country words that appear inside remote wording. Deliberately small: an unrecognised word leaves the scope unrestricted rather than inventing a limit.
const COUNTRY_WORDS: &[(&str, &str)] = &[
    ("usa", "US"),
    ("us", "US"),
    ("united states", "US"),
    ("germany", "DE"),
    ("deutschland", "DE"),
    ("uk", "GB"),
    ("united kingdom", "GB"),
    ("canada", "CA"),
    ("sweden", "SE"),
    ("finland", "FI"),
    ("norway", "NO"),
    ("ireland", "IE"),
    ("netherlands", "NL"),
    ("india", "IN"),
    ("emea", "EU"),
    ("europe", "EU"),
];

/// Score one opportunity against one profile. `semantic` is the cosine similarity from a real embedding provider, or `None` when no provider ran.
pub fn evaluate(
    profile: &Profile,
    candidate: &Candidate,
    rules: &[ExclusionRule],
    semantic: Option<f32>,
) -> MatchResult {
    let haystack = normalize_location(Some(&format!(
        "{} {} {} {}",
        candidate.title,
        candidate.description.as_deref().unwrap_or(""),
        candidate.department.as_deref().unwrap_or(""),
        candidate.organization
    )));
    let title_folded = normalize_location(Some(&candidate.title));
    let description_folded =
        normalize_location(Some(candidate.description.as_deref().unwrap_or("")));

    let mut exclusions = apply_rules(candidate, rules, &title_folded, &description_folded);
    let mut uncertainties = Vec::new();
    let mut factors = Vec::new();
    let mut matched_concepts: Vec<String> = Vec::new();

    // Kind is a hard filter when the profile names kinds at all, because an owner who asked only for theses does not want full-time posts.
    if !profile.job_kinds.is_empty() && !profile.job_kinds.iter().any(|k| k == &candidate.kind) {
        exclusions.push(Exclusion {
            reason: format!(
                "this profile does not look for {} roles",
                candidate.kind.replace('_', " ")
            ),
            evidence: Some(candidate.kind.clone()),
            reversible: true,
        });
    } else if !profile.job_kinds.is_empty() {
        factors.push(Factor {
            name: "job kind".into(),
            contribution: profile.weights.kind,
            detail: format!(
                "{} is a kind this profile looks for",
                candidate.kind.replace('_', " ")
            ),
            evidence: vec![candidate.kind.clone()],
        });
    }

    let location = evaluate_location(profile, candidate);
    match location.verdict {
        LocationVerdict::Excluded(reason) => exclusions.push(Exclusion {
            reason,
            evidence: location.evidence.clone(),
            reversible: true,
        }),
        LocationVerdict::Eligible(detail) => factors.push(Factor {
            name: "location".into(),
            contribution: location.weight * profile.weights.location,
            detail,
            evidence: location.evidence.clone().into_iter().collect(),
        }),
    }
    uncertainties.extend(location.uncertainties);

    // Terms and their concepts. A literal hit and a concept hit are reported separately so the owner can see which of the two carried the match.
    let mut literal_hits: Vec<String> = Vec::new();
    let mut concept_hits: Vec<String> = Vec::new();
    // Wording already credited. Several of your terms can point at the same concept phrase, but the posting only says it once, so it counts once.
    let mut credited: Vec<&str> = Vec::new();
    let mut title_hits: Vec<String> = Vec::new();
    let mut literal_weight = 0.0f32;
    let mut concept_weight = 0.0f32;
    let mut title_weight = 0.0f32;
    for term in &profile.terms {
        let folded = normalize_location(Some(term));
        if folded.is_empty() {
            continue;
        }
        if contains_phrase(&haystack, &folded) {
            literal_weight += phrase_weight(&folded);
            literal_hits.push(term.clone());
            if contains_phrase(&title_folded, &folded) {
                title_weight += phrase_weight(&folded);
                title_hits.push(term.clone());
            }
            continue;
        }
        // The term itself is absent, so try the other ways its concept can be expressed. This is what finds a memory-systems role that never writes DRAM.
        if let Some(concept) = concepts::concept_for(&folded) {
            if let Some(hit) = concepts::phrases_of(concept)
                .iter()
                .find(|p| contains_phrase(&haystack, p))
            {
                if !matched_concepts.contains(&concept.to_string()) {
                    matched_concepts.push(concept.to_string());
                }
                if !credited.contains(hit) {
                    credited.push(hit);
                    concept_weight += phrase_weight(hit);
                    concept_hits.push(format!("{hit} (your term: {term})"));
                    if contains_phrase(&title_folded, hit) {
                        title_weight += phrase_weight(hit);
                        title_hits.push(format!("{hit} in the title"));
                    }
                }
            }
        }
    }

    if !literal_hits.is_empty() {
        factors.push(Factor {
            name: "interest terms".into(),
            contribution: profile.weights.terms * literal_weight,
            detail: format!(
                "{} of your terms appear in this posting",
                literal_hits.len()
            ),
            evidence: literal_hits.clone(),
        });
    }
    if !concept_hits.is_empty() {
        factors.push(Factor {
            name: "related wording".into(),
            contribution: profile.weights.concepts * concept_weight,
            detail: format!(
                "{} related phrases matched even though your exact terms do not appear",
                concept_hits.len()
            ),
            evidence: concept_hits.clone(),
        });
    }
    if !title_hits.is_empty() {
        factors.push(Factor {
            name: "title match".into(),
            contribution: profile.weights.title * title_weight,
            detail: "the match is in the job title, not only the body".into(),
            evidence: title_hits,
        });
    }

    let interests = profile
        .terms
        .iter()
        .filter(|t| !t.trim().is_empty())
        .count();
    if interests > 0 {
        let covered = (literal_hits.len() + concept_hits.len()) as f32 / interests as f32;
        if covered > 0.0 {
            factors.push(Factor {
                name: "interest coverage".into(),
                contribution: profile.weights.terms * 2.0 * covered,
                detail: format!(
                    "{} of your {interests} interests are represented in this posting",
                    literal_hits.len() + concept_hits.len()
                ),
                evidence: Vec::new(),
            });
        }
    }

    let tech_hits: Vec<String> = profile
        .preferred_technologies
        .iter()
        .filter(|t| contains_phrase(&haystack, &normalize_location(Some(t))))
        .cloned()
        .collect();
    if !tech_hits.is_empty() {
        factors.push(Factor {
            name: "preferred technology".into(),
            contribution: profile.weights.technology * tech_hits.len() as f32,
            detail: "technologies you prefer appear in this posting".into(),
            evidence: tech_hits,
        });
    }

    let excluded_tech: Vec<String> = profile
        .excluded_technologies
        .iter()
        .filter(|t| contains_phrase(&haystack, &normalize_location(Some(t))))
        .cloned()
        .collect();
    if !excluded_tech.is_empty() {
        factors.push(Factor {
            name: "unwanted technology".into(),
            contribution: -profile.weights.technology * excluded_tech.len() as f32,
            detail: "technologies you marked as unwanted appear in this posting".into(),
            evidence: excluded_tech,
        });
    }

    let negative_hits: Vec<String> = profile
        .negative_terms
        .iter()
        .filter(|t| contains_phrase(&haystack, &normalize_location(Some(t))))
        .cloned()
        .collect();
    if !negative_hits.is_empty() {
        factors.push(Factor {
            name: "terms you deprioritized".into(),
            contribution: -profile.weights.terms * negative_hits.len() as f32,
            detail: "these count against the match but do not exclude it".into(),
            evidence: negative_hits,
        });
    }

    if let Some(seniority) = evaluate_seniority(profile, &description_folded, &title_folded) {
        match seniority {
            SeniorityOutcome::Excluded(reason, evidence) => exclusions.push(Exclusion {
                reason,
                evidence: Some(evidence),
                reversible: true,
            }),
            SeniorityOutcome::Factor(factor) => factors.push(factor),
        }
    }

    if candidate.title_only {
        uncertainties.push(Uncertainty {
            field: "description".into(),
            detail: "the source published no description, so only the title could be matched"
                .into(),
        });
    }
    if candidate.country_code.is_none() && candidate.city.is_none() {
        uncertainties.push(Uncertainty {
            field: "location".into(),
            detail: "the source did not state a location this tool could read".into(),
        });
    }
    if candidate.advertised_state != "advertised" {
        uncertainties.push(Uncertainty {
            field: "offer".into(),
            detail: format!(
                "this is recorded as {}, so it may not be a funded vacancy",
                candidate.advertised_state.replace('_', " ")
            ),
        });
    }
    if !profile.work_authorization.is_empty() {
        if let Some(country) = candidate.country_code.as_deref() {
            if !profile.work_authorization.iter().any(|c| c == country) {
                uncertainties.push(Uncertainty {
                    field: "work authorization".into(),
                    detail: format!(
                        "you have not recorded work authorization for {country}; the posting's sponsorship policy is unknown and was not assumed"
                    ),
                });
            }
        }
    }

    if let Some(age) = candidate.age_days {
        let freshness = if age <= 7 {
            1.0
        } else if age <= 30 {
            0.5
        } else {
            0.0
        };
        if freshness > 0.0 {
            factors.push(Factor {
                name: "recently seen".into(),
                contribution: profile.weights.freshness * freshness,
                detail: format!("first seen {age} days ago"),
                evidence: Vec::new(),
            });
        }
    }

    let model = match semantic {
        Some(similarity) => {
            factors.push(Factor {
                name: "semantic similarity".into(),
                contribution: profile.weights.semantic * similarity,
                detail: "an embedding model compared this posting with your profile".into(),
                evidence: vec![format!("cosine similarity {similarity:.3}")],
            });
            format!("{LEXICAL_MODEL}+embeddings")
        }
        None => LEXICAL_MODEL.to_owned(),
    };

    let raw: f32 = factors.iter().map(|f| f.contribution).sum();
    // Squashed into 0 to 100 so the number is comparable between opportunities under one profile. It is a ranking aid, not a probability, and the interface says so.
    let score = if raw <= 0.0 {
        0.0
    } else {
        (100.0 * (raw / (raw + 6.0))).clamp(0.0, 100.0)
    };

    MatchResult {
        score: if exclusions.is_empty() { score } else { 0.0 },
        eligible: exclusions.is_empty(),
        exclusions,
        factors,
        uncertainties,
        matched_concepts,
        model,
    }
}

fn apply_rules(
    candidate: &Candidate,
    rules: &[ExclusionRule],
    title_folded: &str,
    description_folded: &str,
) -> Vec<Exclusion> {
    let organization = normalize_organization(&candidate.organization);
    let mut out = Vec::new();
    for rule in rules {
        match rule.kind {
            ExclusionKind::Company => {
                // Exact normalized identity, never a substring: blocking "Core Systems" must not also block "Hardcore Systems".
                if normalize_organization(&rule.value) == organization {
                    out.push(Exclusion {
                        reason: format!("you excluded the employer {}", candidate.organization),
                        evidence: rule.note.clone().or_else(|| Some(rule.value.clone())),
                        reversible: true,
                    });
                }
            }
            ExclusionKind::TitleTerm => {
                let needle = normalize_location(Some(&rule.value));
                if !needle.is_empty() && contains_phrase(title_folded, &needle) {
                    out.push(Exclusion {
                        reason: format!("the title contains '{}', which you excluded", rule.value),
                        evidence: Some(candidate.title.clone()),
                        reversible: true,
                    });
                }
            }
            ExclusionKind::DescriptionTerm => {
                let needle = normalize_location(Some(&rule.value));
                if !needle.is_empty() && contains_phrase(description_folded, &needle) {
                    out.push(Exclusion {
                        reason: format!(
                            "the description contains '{}', which you excluded",
                            rule.value
                        ),
                        evidence: Some(rule.value.clone()),
                        reversible: true,
                    });
                }
            }
            ExclusionKind::Hidden => out.push(Exclusion {
                reason: "you hid this opportunity".into(),
                evidence: rule.note.clone(),
                reversible: true,
            }),
        }
    }
    out
}

enum LocationVerdict {
    Eligible(String),
    Excluded(String),
}

struct LocationOutcome {
    verdict: LocationVerdict,
    weight: f32,
    evidence: Option<String>,
    uncertainties: Vec<Uncertainty>,
}

fn evaluate_location(profile: &Profile, candidate: &Candidate) -> LocationOutcome {
    let scope = remote_scope(
        candidate.remote_raw.as_deref(),
        candidate.country_code.as_deref(),
    );
    let mut uncertainties = Vec::new();
    let evidence = candidate
        .remote_raw
        .clone()
        .or_else(|| candidate.city.clone())
        .or_else(|| candidate.country_code.clone());

    let in_preferred_country = candidate
        .country_code
        .as_deref()
        .is_some_and(|c| profile.preferred_countries.iter().any(|p| p == c));
    let in_preferred_city = candidate.city.as_deref().is_some_and(|c| {
        profile
            .preferred_cities
            .iter()
            .any(|p| p.eq_ignore_ascii_case(c))
    });
    let workable_remotely_now = match &scope {
        RemoteScope::Anywhere => true,
        RemoteScope::RestrictedTo(countries) => profile
            .current_country
            .as_deref()
            .is_some_and(|current| countries.iter().any(|c| c == current)),
        _ => false,
    };

    // A region-locked remote role is not a way to work from somewhere else. Saying so explicitly is what keeps a US-only remote post from looking like a remote option for someone in Germany.
    if let RemoteScope::RestrictedTo(countries) = &scope {
        if !workable_remotely_now {
            uncertainties.push(Uncertainty {
                field: "remote eligibility".into(),
                detail: format!(
                    "remote, but the source restricts it to {}, so it is not workable from {}",
                    countries.join(", "),
                    profile
                        .current_country
                        .as_deref()
                        .unwrap_or("your current country")
                ),
            });
        }
    }

    if profile.remote_policy == RemotePolicy::RemoteFromCurrentCountryOnly && !workable_remotely_now
    {
        return LocationOutcome {
            verdict: LocationVerdict::Excluded(
                "your profile accepts only roles workable remotely from where you are now".into(),
            ),
            weight: 0.0,
            evidence,
            uncertainties,
        };
    }

    if profile.preferred_countries.is_empty() && profile.preferred_cities.is_empty() {
        return LocationOutcome {
            verdict: LocationVerdict::Eligible("this profile does not restrict location".into()),
            weight: 0.0,
            evidence,
            uncertainties,
        };
    }

    if in_preferred_city {
        return LocationOutcome {
            verdict: LocationVerdict::Eligible(format!(
                "{} is one of your preferred cities",
                candidate.city.clone().unwrap_or_default()
            )),
            weight: 2.0,
            evidence,
            uncertainties,
        };
    }
    if in_preferred_country {
        return LocationOutcome {
            verdict: LocationVerdict::Eligible(format!(
                "{} is one of your preferred countries",
                candidate.country_code.clone().unwrap_or_default()
            )),
            weight: 1.0,
            evidence,
            uncertainties,
        };
    }
    if workable_remotely_now {
        return LocationOutcome {
            verdict: LocationVerdict::Eligible("remote and workable from where you are now".into()),
            weight: 1.0,
            evidence,
            uncertainties,
        };
    }
    if candidate.country_code.is_none() && candidate.city.is_none() {
        // A location the parser could not read is a gap in this tool, not a reason to hide a real opening.
        return if profile.include_unknown_location {
            LocationOutcome {
                verdict: LocationVerdict::Eligible(
                    "location unknown, kept because you asked to see these".into(),
                ),
                weight: 0.0,
                evidence,
                uncertainties,
            }
        } else {
            LocationOutcome {
                verdict: LocationVerdict::Excluded(
                    "location unknown and your profile hides these".into(),
                ),
                weight: 0.0,
                evidence,
                uncertainties,
            }
        };
    }
    if profile.relocation_willing
        && profile.remote_policy != RemotePolicy::RemoteFromCurrentCountryOnly
    {
        return LocationOutcome {
            verdict: LocationVerdict::Eligible(format!(
                "outside your preferred locations, kept because you are willing to relocate to {}",
                candidate
                    .city
                    .clone()
                    .or_else(|| candidate.country_code.clone())
                    .unwrap_or_else(|| "this location".into())
            )),
            weight: 0.5,
            evidence,
            uncertainties,
        };
    }
    LocationOutcome {
        verdict: LocationVerdict::Excluded(format!(
            "{} is outside your preferred locations and relocation is off",
            candidate
                .city
                .clone()
                .or_else(|| candidate.country_code.clone())
                .unwrap_or_else(|| "this location".into())
        )),
        weight: 0.0,
        evidence,
        uncertainties,
    }
}

enum SeniorityOutcome {
    Excluded(String, String),
    Factor(Factor),
}

/// Read an advertised years-of-experience requirement. Only an explicit number counts; a posting that says nothing produces nothing.
fn evaluate_seniority(
    profile: &Profile,
    description: &str,
    title: &str,
) -> Option<SeniorityOutcome> {
    if profile.seniority_policy == SeniorityPolicy::Ignore {
        return None;
    }
    let max_wanted = profile.max_years_experience?;
    let advertised = advertised_years(description).or_else(|| advertised_years(title))?;
    if advertised <= max_wanted {
        return Some(SeniorityOutcome::Factor(Factor {
            name: "experience level".into(),
            contribution: 0.5,
            detail: format!("asks for {advertised} years, within your range"),
            evidence: vec![format!("{advertised}+ years")],
        }));
    }
    match profile.seniority_policy {
        SeniorityPolicy::Hard => Some(SeniorityOutcome::Excluded(
            format!("asks for {advertised} years of experience, above the {max_wanted} you set as a hard limit"),
            format!("{advertised}+ years"),
        )),
        // The default: an employer's wish list moves the score and is shown, but does not hide the role.
        _ => Some(SeniorityOutcome::Factor(Factor {
            name: "experience level".into(),
            contribution: -1.0,
            detail: format!(
                "asks for {advertised} years, above your preferred {max_wanted}; shown anyway because this profile treats experience as a preference"
            ),
            evidence: vec![format!("{advertised}+ years")],
        })),
    }
}

fn advertised_years(folded: &str) -> Option<i32> {
    let tokens: Vec<&str> = folded.split(' ').collect();
    for (index, token) in tokens.iter().enumerate() {
        if !matches!(*token, "years" | "year" | "jahre" | "jahren") {
            continue;
        }
        // Look just behind the word for a number, which covers "5 years", "5+ years" and "mindestens 3 Jahre".
        for back in 1..=2 {
            if index >= back {
                if let Ok(value) = tokens[index - back].parse::<i32>() {
                    if (0..=60).contains(&value) {
                        return Some(value);
                    }
                }
            }
        }
    }
    None
}

/// How much one phrase match is worth. A multi-word phrase is far stronger evidence than a single common word: "memory tiering" appearing is informative, "cache" appearing is barely. Without corpus statistics this length proxy is the cheapest honest way to separate a specific hit from a generic one, and it is what keeps hundreds of postings from landing on the same score.
fn phrase_weight(phrase: &str) -> f32 {
    let words = phrase.split(' ').filter(|w| !w.is_empty()).count().max(1);
    1.0 + 0.6 * (words - 1) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A neutral architecture profile. Nothing here describes a real person; the values are the technical defaults a reader would expect from the shipped example file.
    fn architecture_profile() -> Profile {
        let mut profile = Profile::blank("architecture", "Computer architecture");
        profile.terms = vec!["DRAM".into(), "memory tiering".into(), "RISC-V".into()];
        profile.preferred_countries = vec!["DE".into(), "SE".into()];
        profile.preferred_cities = vec!["Munich".into(), "Stockholm".into()];
        profile.current_country = Some("DE".into());
        profile.relocation_willing = true;
        profile
    }

    fn candidate(title: &str, description: &str) -> Candidate {
        Candidate {
            title: title.into(),
            organization: "Example Silicon".into(),
            description: Some(description.into()),
            kind: "employment".into(),
            advertised_state: "advertised".into(),
            country_code: Some("DE".into()),
            city: Some("Munich".into()),
            ..Candidate::default()
        }
    }

    #[test]
    fn a_memory_role_matches_without_ever_using_the_word_dram() {
        let profile = architecture_profile();
        let job = candidate(
            "Systems Software Engineer, Tiered Memory",
            "Design page migration policies across CXL-attached capacity and measure them on real hardware.",
        );
        let result = evaluate(&profile, &job, &[], None);
        assert!(result.eligible);
        assert!(
            result.score > 0.0,
            "an implicit memory role must score above zero"
        );
        assert!(
            result
                .matched_concepts
                .contains(&"memory systems".to_string()),
            "expected the memory systems concept, got {:?}",
            result.matched_concepts
        );
        let related = result
            .factors
            .iter()
            .find(|f| f.name == "related wording")
            .expect("concept factor");
        assert!(
            related
                .evidence
                .iter()
                .any(|e| e.contains("page migration") || e.contains("cxl")),
            "the explanation must name the phrase that matched: {:?}",
            related.evidence
        );
    }

    #[test]
    fn one_phrase_in_a_posting_is_credited_once_however_many_terms_point_at_it() {
        // Five firmware terms all map to the same concept, and the posting says "bring up" once. Crediting it five times would rank this above a posting that genuinely matches five different things.
        let mut many = Profile::blank("firmware", "Firmware and platform");
        many.terms = vec![
            "firmware".into(),
            "RTOS".into(),
            "embedded C".into(),
            "device driver".into(),
            "bootloader".into(),
        ];
        let mut one = Profile::blank("firmware", "Firmware and platform");
        one.terms = vec!["firmware".into()];

        let job = candidate(
            "Bring up Engineer",
            "Board bring up for new silicon, from first power on to a booting system.",
        );
        let with_many = evaluate(&many, &job, &[], None);
        let with_one = evaluate(&one, &job, &[], None);

        let evidence = |result: &MatchResult, factor: &str| {
            result
                .factors
                .iter()
                .find(|f| f.name == factor)
                .map(|f| f.evidence.clone())
                .unwrap_or_default()
        };
        assert_eq!(
            evidence(&with_many, "related wording").len(),
            1,
            "the posting said it once: {:?}",
            evidence(&with_many, "related wording")
        );
        let titles = evidence(&with_many, "title match");
        let mut unique = titles.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), titles.len(), "repeated evidence: {titles:?}");
        let factor = |result: &MatchResult, name: &str| {
            result
                .factors
                .iter()
                .find(|f| f.name == name)
                .map(|f| f.contribution)
                .unwrap_or(0.0)
        };
        assert!(
            (factor(&with_many, "related wording") - factor(&with_one, "related wording")).abs()
                < f32::EPSILON,
            "the same wording must weigh the same whether one term or five point at it"
        );
    }

    #[test]
    fn a_platform_firmware_role_matches_without_the_word_firmware_in_the_title() {
        let mut profile = Profile::blank("firmware", "Firmware and platform");
        profile.terms = vec!["firmware".into(), "RTOS".into()];
        let job = candidate(
            "Low Level Platform Engineer",
            "Own the RTKit IPC layer, the AOP power sequencing and SMC handoff during board bring up.",
        );
        let result = evaluate(&profile, &job, &[], None);
        assert!(result.eligible);
        assert!(result.score > 0.0);
        assert!(result.matched_concepts.contains(&"firmware".to_string()));
    }

    #[test]
    fn a_german_thesis_and_an_fpga_research_assistant_are_not_dropped_by_a_software_only_profile() {
        let mut profile = Profile::blank("research", "Research and thesis");
        profile.terms = vec!["Masterarbeit".into(), "FPGA".into()];
        profile.job_kinds = vec!["master_thesis".into(), "research_assistant".into()];

        let mut thesis = candidate(
            "Masterarbeit: Seitenmigration in heterogenen Speichersystemen",
            "Abschlussarbeit am Lehrstuhl fuer Rechnerarchitektur.",
        );
        thesis.kind = "master_thesis".into();
        thesis.advertised_state = "open_topic".into();
        let thesis_result = evaluate(&profile, &thesis, &[], None);
        assert!(thesis_result.eligible, "{:?}", thesis_result.exclusions);
        assert!(thesis_result.score > 0.0);
        assert!(
            thesis_result
                .uncertainties
                .iter()
                .any(|u| u.field == "offer"),
            "an open topic must be flagged as possibly unfunded"
        );

        let mut fpga = candidate(
            "FPGA Research Assistant",
            "Maintain the emulation flow, partition RTL and close timing.",
        );
        fpga.kind = "research_assistant".into();
        let fpga_result = evaluate(&profile, &fpga, &[], None);
        assert!(fpga_result.eligible, "{:?}", fpga_result.exclusions);
        assert!(
            fpga_result.score > 0.0,
            "an FPGA assistant must not be filtered out as non-software"
        );
    }

    #[test]
    fn a_physical_stockholm_role_survives_for_a_candidate_in_germany_with_relocation_on() {
        let profile = architecture_profile();
        let mut job = candidate("Memory Systems Engineer", "Work on DRAM controllers.");
        job.country_code = Some("SE".into());
        job.city = Some("Stockholm".into());
        let result = evaluate(&profile, &job, &[], None);
        assert!(result.eligible, "{:?}", result.exclusions);
        let location = result
            .factors
            .iter()
            .find(|f| f.name == "location")
            .expect("location factor");
        assert!(
            location.detail.contains("preferred cities"),
            "{}",
            location.detail
        );

        // The same role with relocation off and Sweden not preferred is honestly excluded rather than silently ranked low.
        let mut narrow = architecture_profile();
        narrow.relocation_willing = false;
        narrow.preferred_countries = vec!["DE".into()];
        narrow.preferred_cities = vec!["Munich".into()];
        let refused = evaluate(&narrow, &job, &[], None);
        assert!(!refused.eligible);
        assert!(refused.exclusions[0]
            .reason
            .contains("outside your preferred locations"));
        assert!(
            refused.exclusions[0].reversible,
            "the owner can lift this by editing the profile"
        );
    }

    #[test]
    fn a_us_only_remote_role_is_not_presented_as_workable_from_germany() {
        let profile = architecture_profile();
        let mut job = candidate("Site Reliability Engineer", "Keep the fleet healthy.");
        job.country_code = Some("US".into());
        job.city = None;
        job.remote_raw = Some("Remote - US only".into());

        let result = evaluate(&profile, &job, &[], None);
        assert!(
            result
                .uncertainties
                .iter()
                .any(|u| u.field == "remote eligibility"
                    && u.detail.contains("not workable from DE")),
            "a region-locked remote role must say it cannot be worked from here: {:?}",
            result.uncertainties
        );
        assert!(
            !result
                .factors
                .iter()
                .any(|f| f.detail.contains("workable from where you are now")),
            "it must never be credited as remote-from-here"
        );

        // A profile that only accepts remote work from the current country excludes it outright.
        let mut remote_only = architecture_profile();
        remote_only.remote_policy = RemotePolicy::RemoteFromCurrentCountryOnly;
        let refused = evaluate(&remote_only, &job, &[], None);
        assert!(!refused.eligible);

        // A genuinely unrestricted remote role is workable from here.
        job.remote_raw = Some("Remote".into());
        let open = evaluate(&remote_only, &job, &[], None);
        assert!(open.eligible, "{:?}", open.exclusions);
    }

    #[test]
    fn remote_wording_is_read_only_as_far_as_it_actually_states() {
        assert_eq!(remote_scope(None, Some("US")), RemoteScope::Unstated);
        assert_eq!(
            remote_scope(Some("Munich, Germany"), Some("DE")),
            RemoteScope::Unstated
        );
        assert_eq!(
            remote_scope(Some("Remote"), Some("US")),
            RemoteScope::Anywhere
        );
        assert_eq!(
            remote_scope(Some("Hybrid"), Some("DE")),
            RemoteScope::Hybrid
        );
        assert_eq!(
            remote_scope(Some("Remote - US only"), Some("US")),
            RemoteScope::RestrictedTo(vec!["US".into()])
        );
        assert_eq!(
            remote_scope(Some("Remote (Germany)"), None),
            RemoteScope::RestrictedTo(vec!["DE".into()])
        );
        // "Only" with no named place falls back to the country the source attached, and nothing more is invented.
        assert_eq!(
            remote_scope(
                Some("Remote, this role is onsite-optional only"),
                Some("CA")
            ),
            RemoteScope::RestrictedTo(vec!["CA".into()])
        );
    }

    #[test]
    fn a_company_exclusion_blocks_an_exact_alias_and_never_an_unrelated_substring() {
        let profile = architecture_profile();
        let rules = vec![ExclusionRule {
            kind: ExclusionKind::Company,
            value: "Example Silicon GmbH".into(),
            note: Some("not hiring in my area".into()),
        }];
        let blocked = evaluate(
            &profile,
            &candidate("Memory Engineer", "DRAM work"),
            &rules,
            None,
        );
        assert!(
            !blocked.eligible,
            "a normalized alias of the same employer must be blocked"
        );
        assert!(blocked.exclusions[0]
            .reason
            .contains("excluded the employer"));

        let mut other = candidate("Memory Engineer", "DRAM work");
        other.organization = "Hardcore Example Silicon Systems".into();
        let allowed = evaluate(&profile, &other, &rules, None);
        assert!(
            allowed.eligible,
            "an unrelated employer sharing words must not be blocked"
        );
    }

    #[test]
    fn scoped_keyword_exclusions_apply_only_to_the_field_they_name() {
        let profile = architecture_profile();
        let title_rule = vec![ExclusionRule {
            kind: ExclusionKind::TitleTerm,
            value: "sales".into(),
            note: None,
        }];
        let sales = evaluate(
            &profile,
            &candidate("Sales Engineer, Memory", "DRAM"),
            &title_rule,
            None,
        );
        assert!(!sales.eligible);
        // The same word in the body does not trip a title-scoped rule.
        let body = evaluate(
            &profile,
            &candidate(
                "Memory Systems Engineer",
                "You will support the sales team with DRAM benchmarks.",
            ),
            &title_rule,
            None,
        );
        assert!(body.eligible, "{:?}", body.exclusions);
    }

    #[test]
    fn an_advertised_experience_level_is_a_preference_unless_the_profile_makes_it_hard() {
        let mut profile = architecture_profile();
        profile.max_years_experience = Some(3);
        let senior = candidate(
            "Principal Memory Architect",
            "We are looking for 10 years of experience in DRAM subsystem design.",
        );

        let preference = evaluate(&profile, &senior, &[], None);
        assert!(
            preference.eligible,
            "an employer wish list must not hide the role by default"
        );
        let factor = preference
            .factors
            .iter()
            .find(|f| f.name == "experience level")
            .expect("factor");
        assert!(
            factor.contribution < 0.0,
            "it should count against the score"
        );

        profile.seniority_policy = SeniorityPolicy::Hard;
        let hard = evaluate(&profile, &senior, &[], None);
        assert!(!hard.eligible);
        assert!(hard.exclusions[0].reason.contains("hard limit"));

        profile.seniority_policy = SeniorityPolicy::Ignore;
        let ignored = evaluate(&profile, &senior, &[], None);
        assert!(ignored.eligible);
        assert!(!ignored.factors.iter().any(|f| f.name == "experience level"));
    }

    #[test]
    fn an_opportunity_with_unreadable_metadata_is_kept_and_its_gaps_are_named() {
        let profile = architecture_profile();
        let mut sparse = Candidate {
            title: "Memory Systems Engineer".into(),
            organization: "Example Silicon".into(),
            kind: "employment".into(),
            advertised_state: "advertised".into(),
            title_only: true,
            ..Candidate::default()
        };
        sparse.description = None;

        let result = evaluate(&profile, &sparse, &[], None);
        assert!(
            result.eligible,
            "a parsing gap must not hide a real role: {:?}",
            result.exclusions
        );
        assert!(result
            .uncertainties
            .iter()
            .any(|u| u.field == "description"));
        assert!(result.uncertainties.iter().any(|u| u.field == "location"));

        // A profile that asks to hide unknown locations gets what it asked for, with the reason stated.
        let mut strict = architecture_profile();
        strict.include_unknown_location = false;
        let hidden = evaluate(&strict, &sparse, &[], None);
        assert!(!hidden.eligible);
        assert!(hidden.exclusions[0].reason.contains("location unknown"));
    }

    #[test]
    fn the_scorer_names_itself_honestly_and_never_claims_ai_without_a_model() {
        let profile = architecture_profile();
        let job = candidate("Memory Systems Engineer", "DRAM and CXL work.");

        let lexical = evaluate(&profile, &job, &[], None);
        assert_eq!(lexical.model, LEXICAL_MODEL);
        assert!(!lexical.model.to_lowercase().contains("ai"));
        assert!(!lexical
            .factors
            .iter()
            .any(|f| f.name == "semantic similarity"));

        let with_model = evaluate(&profile, &job, &[], Some(0.82));
        assert_eq!(with_model.model, format!("{LEXICAL_MODEL}+embeddings"));
        let semantic = with_model
            .factors
            .iter()
            .find(|f| f.name == "semantic similarity")
            .expect("factor");
        assert!(semantic.evidence[0].contains("0.82"));
        assert!(
            with_model.score > lexical.score,
            "a real similarity should lift the score"
        );
    }

    #[test]
    fn an_excluded_opportunity_scores_zero_and_explains_itself_rather_than_vanishing() {
        let profile = architecture_profile();
        let rules = vec![ExclusionRule {
            kind: ExclusionKind::Hidden,
            value: String::new(),
            note: Some("already applied".into()),
        }];
        let result = evaluate(
            &profile,
            &candidate("Memory Engineer", "DRAM"),
            &rules,
            None,
        );
        assert!(!result.eligible);
        assert_eq!(result.score, 0.0);
        assert_eq!(result.exclusions.len(), 1);
        assert!(result.exclusions[0].reversible);
        // The factors are still computed, so the owner can see what it would have scored.
        assert!(!result.factors.is_empty());
    }

    #[test]
    fn a_job_kind_the_profile_does_not_want_is_excluded_with_a_reversible_reason() {
        let mut profile = architecture_profile();
        profile.job_kinds = vec!["master_thesis".into()];
        let result = evaluate(
            &profile,
            &candidate("Memory Systems Engineer", "DRAM"),
            &[],
            None,
        );
        assert!(!result.eligible);
        assert!(result.exclusions[0]
            .reason
            .contains("does not look for employment"));
        assert!(result.exclusions[0].reversible);
    }

    #[test]
    fn a_specific_phrase_outweighs_a_generic_single_word() {
        assert!(phrase_weight("memory tiering") > phrase_weight("cache"));
        assert!(phrase_weight("transient execution attack") > phrase_weight("memory tiering"));
        assert_eq!(phrase_weight("cache"), 1.0);
    }

    #[test]
    fn two_postings_hitting_different_numbers_of_interests_do_not_tie() {
        let mut profile = architecture_profile();
        profile.terms = vec![
            "DRAM".into(),
            "memory tiering".into(),
            "RISC-V".into(),
            "cache".into(),
        ];
        let broad = evaluate(
            &profile,
            &candidate(
                "Memory Systems Engineer",
                "Work on DRAM, memory tiering, RISC-V and cache design.",
            ),
            &[],
            None,
        );
        let narrow = evaluate(
            &profile,
            &candidate("Systems Engineer", "Some cache work."),
            &[],
            None,
        );
        let none = evaluate(
            &profile,
            &candidate("Office Manager", "Keep the office running."),
            &[],
            None,
        );
        assert!(
            broad.score > narrow.score,
            "{} should beat {}",
            broad.score,
            narrow.score
        );
        assert!(
            narrow.score > none.score,
            "{} should beat {}",
            narrow.score,
            none.score
        );
        // The coverage factor is what separates a posting matching all four interests from one matching a single generic word.
        assert!(broad.factors.iter().any(|f| f.name == "interest coverage"));
    }

    #[test]
    fn scores_rank_a_closer_match_above_a_weaker_one() {
        let profile = architecture_profile();
        let strong = evaluate(
            &profile,
            &candidate(
                "DRAM Memory Systems Engineer",
                "RISC-V memory tiering and page migration.",
            ),
            &[],
            None,
        );
        let weak = evaluate(
            &profile,
            &candidate("Office Manager", "Keep the Munich office running."),
            &[],
            None,
        );
        assert!(
            strong.score > weak.score,
            "{} should beat {}",
            strong.score,
            weak.score
        );
        assert!(strong.score <= 100.0 && weak.score >= 0.0);
    }
}
