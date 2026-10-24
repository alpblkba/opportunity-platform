//! Academic listing semantics. A university page mixes three different things that must never be conflated: a funded advertised vacancy, a thesis topic a student may claim, and a professor who might supervise something. Everything here is extraction from published text; nothing is inferred about availability or funding.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};

use crate::normalize::normalize_location;
use crate::types::AdvertisedState;

/// Wording that marks a record as a topic on offer rather than a paid vacancy. A thesis topic is advertised as work a student may take on, not as a position with a salary.
const OPEN_TOPIC_PHRASES: &[&str] = &[
    "masterarbeit",
    "master arbeit",
    "bachelorarbeit",
    "abschlussarbeit",
    "studienarbeit",
    "diplomarbeit",
    "forschungsarbeit",
    "projektarbeit",
    "master thesis",
    "masters thesis",
    "master s thesis",
    "bachelor thesis",
    "thesis topic",
    "thesis project",
    "offene arbeit",
    "offene arbeiten",
    "open topic",
    "open topics",
    "topic available",
    "themenangebot",
];

/// Wording that marks a page as a person who may supervise work, with nothing on offer yet.
const SUPERVISOR_PAGE_PHRASES: &[&str] = &[
    "potential supervisor",
    "prospective supervisor",
    "research interests",
    "forschungsinteressen",
    "betreuungsangebot",
    "supervision enquiries",
    "contact for theses",
];

/// Labels that introduce a named supervisor in German or English listing text.
const SUPERVISOR_LABELS: &[&str] = &[
    "betreuer:in",
    "betreuerin",
    "betreuer",
    "betreuung",
    "supervisor",
    "advisor",
    "adviser",
    "ansprechpartner:in",
    "ansprechpartnerin",
    "ansprechpartner",
    "kontaktperson",
];

/// Labels that introduce an application deadline.
const DEADLINE_LABELS: &[&str] = &[
    "bewerbungsfrist",
    "bewerbungsschluss",
    "frist",
    "einsendeschluss",
    "deadline",
    "application deadline",
    "closing date",
    "apply by",
];

/// Decide what an academic record is actually offering. Defaults to `Unknown` rather than `Advertised`, because a university page that says nothing about funding has not said a role is funded.
pub fn academic_state(title: &str, body: Option<&str>) -> AdvertisedState {
    // Folded, not title-normalized: the title normalizer deliberately cuts everything after the first parenthesis, which would discard the body of a role titled "... (m/w/d)".
    let haystack = normalize_location(Some(&format!("{} {}", title, body.unwrap_or_default())));
    if SUPERVISOR_PAGE_PHRASES
        .iter()
        .any(|p| contains_phrase(&haystack, p))
    {
        return AdvertisedState::PotentialSupervisor;
    }
    if OPEN_TOPIC_PHRASES
        .iter()
        .any(|p| contains_phrase(&haystack, p))
    {
        return AdvertisedState::OpenTopic;
    }
    // A salaried academic post states its pay scale, which is the one reliable signal that it is a funded vacancy rather than a topic.
    if [
        "tv l",
        "tvoed",
        "tv oed",
        "e13",
        "e 13",
        "salary",
        "gehalt",
        "verguetung",
        "entgeltgruppe",
    ]
    .iter()
    .any(|p| contains_phrase(&haystack, p))
    {
        return AdvertisedState::Advertised;
    }
    AdvertisedState::Unknown
}

/// Pull a named supervisor out of listing text. Returns `None` unless the text actually labels someone, because guessing a professor's name onto a topic would invent a commitment that was never made.
pub fn extract_supervisor(text: &str) -> Option<String> {
    for line in text.lines() {
        let lower = line.to_lowercase();
        for label in SUPERVISOR_LABELS {
            let Some(at) = lower.find(label) else {
                continue;
            };
            let after = &line[at + label.len()..];
            let value = after
                .trim_start_matches([':', '-', '\u{2013}', ' ', '\t'])
                .trim();
            let value = value.split(['|', ';', ',']).next().unwrap_or(value).trim();
            if value.len() >= 3 && value.len() <= 120 && value.chars().any(char::is_alphabetic) {
                return Some(value.to_owned());
            }
        }
    }
    None
}

/// Pull an application deadline out of listing text. Understands the German `31.12.2026` and `31. Dezember 2026` forms alongside ISO dates.
pub fn extract_deadline(text: &str) -> Option<DateTime<Utc>> {
    for line in text.lines() {
        let lower = line.to_lowercase();
        if !DEADLINE_LABELS.iter().any(|label| lower.contains(label)) {
            continue;
        }
        if let Some(date) = parse_any_date(line) {
            return Some(date);
        }
    }
    None
}

const GERMAN_MONTHS: &[(&str, u32)] = &[
    ("januar", 1),
    ("februar", 2),
    ("maerz", 3),
    ("marz", 3),
    ("april", 4),
    ("mai", 5),
    ("juni", 6),
    ("juli", 7),
    ("august", 8),
    ("september", 9),
    ("oktober", 10),
    ("november", 11),
    ("dezember", 12),
];

fn parse_any_date(line: &str) -> Option<DateTime<Utc>> {
    let digits: Vec<String> = line
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();

    // ISO first: 2026-12-31.
    if let Some(found) = find_iso(line) {
        return Some(found);
    }
    // German numeric: 31.12.2026 or 31.12.26.
    for window in digits.windows(3) {
        let (Ok(d), Ok(m), Ok(y)) = (
            window[0].parse::<u32>(),
            window[1].parse::<u32>(),
            window[2].parse::<i32>(),
        ) else {
            continue;
        };
        let year = if (0..=99).contains(&y) { 2000 + y } else { y };
        if (1..=31).contains(&d) && (1..=12).contains(&m) && (2000..=2100).contains(&year) {
            if let Some(date) = NaiveDate::from_ymd_opt(year, m, d) {
                return Some(Utc.from_utc_datetime(&date.and_hms_opt(23, 59, 59)?));
            }
        }
    }
    // German written month: 31. Dezember 2026.
    let folded = crate::normalize::normalize_location(Some(line));
    for (name, month) in GERMAN_MONTHS {
        let Some(at) = folded.find(name) else {
            continue;
        };
        let before: Vec<&str> = folded[..at].split_whitespace().collect();
        let after: Vec<&str> = folded[at + name.len()..].split_whitespace().collect();
        let day = before.last().and_then(|t| t.parse::<u32>().ok())?;
        let year = after.first().and_then(|t| t.parse::<i32>().ok())?;
        if (1..=31).contains(&day) && (2000..=2100).contains(&year) {
            if let Some(date) = NaiveDate::from_ymd_opt(year, *month, day) {
                return Some(Utc.from_utc_datetime(&date.and_hms_opt(23, 59, 59)?));
            }
        }
    }
    None
}

fn find_iso(line: &str) -> Option<DateTime<Utc>> {
    let chars: Vec<char> = line.chars().collect();
    for start in 0..chars.len().saturating_sub(9) {
        // The window must be a whole token: chrono accepts leading whitespace and a short day, so an unanchored scan would read "2026-11-3" out of "2026-11-30".
        if !chars[start].is_ascii_digit() || (start > 0 && chars[start - 1].is_ascii_digit()) {
            continue;
        }
        if chars.get(start + 10).is_some_and(|c| c.is_ascii_digit()) {
            continue;
        }
        let slice: String = chars[start..start + 10].iter().collect();
        if let Ok(date) = NaiveDate::parse_from_str(&slice, "%Y-%m-%d") {
            return Some(Utc.from_utc_datetime(&date.and_hms_opt(23, 59, 59)?));
        }
    }
    None
}

fn contains_phrase(haystack: &str, phrase: &str) -> bool {
    let hay: Vec<&str> = haystack.split(' ').collect();
    let needle: Vec<&str> = phrase.split(' ').collect();
    !needle.is_empty()
        && needle.len() <= hay.len()
        && hay.windows(needle.len()).any(|w| w == needle.as_slice())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    #[test]
    fn a_german_thesis_topic_is_an_open_topic_not_an_advertised_vacancy() {
        assert_eq!(
            academic_state("Masterarbeit: Speicherhierarchien für RISC-V", None),
            AdvertisedState::OpenTopic
        );
        assert_eq!(
            academic_state("Abschlussarbeit Seitenmigration", None),
            AdvertisedState::OpenTopic
        );
        assert_eq!(
            academic_state("Open topic: near-memory compute", None),
            AdvertisedState::OpenTopic
        );
    }

    #[test]
    fn a_funded_post_that_states_its_pay_scale_is_advertised() {
        let body = "Befristete Stelle, Entgeltgruppe 13 TV-L, 100 Prozent.";
        assert_eq!(
            academic_state("Wissenschaftliche Mitarbeiterin", Some(body)),
            AdvertisedState::Advertised
        );
        // A gendered title suffix must not truncate the body before the pay scale is seen.
        assert_eq!(
            academic_state(
                "Wissenschaftliche Mitarbeiterin (m/w/d) Speichersysteme",
                Some(body)
            ),
            AdvertisedState::Advertised
        );
    }

    #[test]
    fn a_supervisor_page_never_implies_an_available_position() {
        let state = academic_state(
            "Prof. Dr. Example",
            Some("Forschungsinteressen: Speichersysteme, RISC-V"),
        );
        assert_eq!(state, AdvertisedState::PotentialSupervisor);
        let state = academic_state(
            "Prospective supervisor",
            Some("Contact for theses in memory systems"),
        );
        assert_eq!(state, AdvertisedState::PotentialSupervisor);
    }

    #[test]
    fn a_record_that_says_nothing_about_funding_stays_unknown() {
        assert_eq!(
            academic_state("Research position", Some("Join our group.")),
            AdvertisedState::Unknown
        );
    }

    #[test]
    fn a_named_supervisor_is_extracted_in_german_and_english() {
        assert_eq!(
            extract_supervisor("Betreuer: Prof. Dr. Anna Beispiel"),
            Some("Prof. Dr. Anna Beispiel".into())
        );
        assert_eq!(
            extract_supervisor("Supervisor: Dr. John Example | room 3.14"),
            Some("Dr. John Example".into())
        );
        assert_eq!(
            extract_supervisor("Ansprechpartner:in: M. Sc. Lena Muster"),
            Some("M. Sc. Lena Muster".into())
        );
    }

    #[test]
    fn a_listing_without_a_named_supervisor_yields_none_rather_than_a_guess() {
        assert_eq!(
            extract_supervisor("Masterarbeit zu Speichersystemen am Institut."),
            None
        );
        assert_eq!(
            extract_supervisor("Prof. Dr. Anna Beispiel forscht zu DRAM."),
            None
        );
    }

    #[test]
    fn deadlines_are_parsed_from_german_and_iso_forms() {
        let german = extract_deadline("Bewerbungsfrist: 31.12.2026").unwrap();
        assert_eq!(
            (german.year(), german.month(), german.day()),
            (2026, 12, 31)
        );
        let written = extract_deadline("Bewerbungsschluss: 15. Januar 2027").unwrap();
        assert_eq!(
            (written.year(), written.month(), written.day()),
            (2027, 1, 15)
        );
        let iso = extract_deadline("Application deadline: 2026-11-30 at 23:59").unwrap();
        assert_eq!((iso.year(), iso.month(), iso.day()), (2026, 11, 30));
        let slash = extract_deadline("Deadline: 01.02.2027").unwrap();
        assert_eq!((slash.year(), slash.month(), slash.day()), (2027, 2, 1));
    }

    #[test]
    fn an_absent_deadline_stays_absent_instead_of_defaulting() {
        assert_eq!(
            extract_deadline("Masterarbeit zu Speichersystemen. Beginn jederzeit moeglich."),
            None
        );
        // A date that is not labelled as a deadline is not a deadline.
        assert_eq!(extract_deadline("Veroeffentlicht am 01.03.2026"), None);
    }
}
