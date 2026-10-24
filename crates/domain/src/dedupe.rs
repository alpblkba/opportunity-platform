use sha2::{Digest, Sha256};

use crate::normalize::{normalize_location, normalize_organization, normalize_title};

/// Grouping key for postings that describe the same real opening across different sources. Deliberately conservative: organization, title and stated location must all agree after normalization. A posting whose location the source did not state gets an empty location component, so it groups only with other location-less postings for the same role and never silently absorbs a differently located one.
pub fn dedupe_key(organization: &str, title: &str, location: Option<&str>) -> String {
    let parts = [
        normalize_organization(organization),
        normalize_title(title),
        normalize_location(location),
    ];
    let mut hasher = Sha256::new();
    for part in &parts {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    format!("{:x}", hasher.finalize())
}

/// Stable content fingerprint used to decide whether a re-fetched posting actually changed. Only fields a source can legitimately revise take part; `last_seen` style metadata is excluded so an unchanged posting does not generate a revision on every scan.
pub fn content_hash(
    title: &str,
    description: Option<&str>,
    location: Option<&str>,
    url: &str,
) -> String {
    let mut hasher = Sha256::new();
    for part in [Some(title), description, location, Some(url)] {
        hasher.update(part.unwrap_or("").as_bytes());
        hasher.update([0u8]);
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_role_on_two_boards_maps_to_one_key() {
        let greenhouse = dedupe_key(
            "Example Systems GmbH",
            "Firmware Engineer (m/w/d)",
            Some("Munich, Germany"),
        );
        let lever = dedupe_key(
            "EXAMPLE SYSTEMS, Inc.",
            "Firmware Engineer",
            Some("munich germany"),
        );
        assert_eq!(greenhouse, lever);
    }

    #[test]
    fn differing_locations_stay_distinct_rather_than_merging_ambiguously() {
        let munich = dedupe_key("Example Systems", "Firmware Engineer", Some("Munich"));
        let stockholm = dedupe_key("Example Systems", "Firmware Engineer", Some("Stockholm"));
        let unstated = dedupe_key("Example Systems", "Firmware Engineer", None);
        assert_ne!(munich, stockholm);
        assert_ne!(munich, unstated);
        assert_ne!(stockholm, unstated);
    }

    #[test]
    fn unrelated_employers_sharing_a_substring_do_not_merge() {
        assert_ne!(
            dedupe_key("Core Systems", "Firmware Engineer", Some("Munich")),
            dedupe_key("Hardcore Systems", "Firmware Engineer", Some("Munich"))
        );
    }

    #[test]
    fn content_hash_changes_only_when_revisable_content_changes() {
        let before = content_hash(
            "Firmware Engineer",
            Some("Build firmware"),
            Some("Munich"),
            "https://example.org/1",
        );
        let same = content_hash(
            "Firmware Engineer",
            Some("Build firmware"),
            Some("Munich"),
            "https://example.org/1",
        );
        let edited = content_hash(
            "Firmware Engineer",
            Some("Build better firmware"),
            Some("Munich"),
            "https://example.org/1",
        );
        assert_eq!(before, same);
        assert_ne!(before, edited);
    }
}
