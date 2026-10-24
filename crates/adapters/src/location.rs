/// Country names and codes that appear in the boards this tool reads. The table is small on purpose: a location string that is not in it yields `None`, which the UI shows as unknown, rather than a guess.
const COUNTRIES: &[(&str, &str)] = &[
    ("germany", "DE"),
    ("deutschland", "DE"),
    ("sweden", "SE"),
    ("sverige", "SE"),
    ("finland", "FI"),
    ("suomi", "FI"),
    ("norway", "NO"),
    ("norge", "NO"),
    ("denmark", "DK"),
    ("netherlands", "NL"),
    ("ireland", "IE"),
    ("united kingdom", "GB"),
    ("uk", "GB"),
    ("united states", "US"),
    ("united states of america", "US"),
    ("usa", "US"),
    ("u s a", "US"),
    ("canada", "CA"),
    ("switzerland", "CH"),
    ("austria", "AT"),
    ("france", "FR"),
    ("spain", "ES"),
    ("italy", "IT"),
    ("poland", "PL"),
    ("israel", "IL"),
    ("india", "IN"),
    ("japan", "JP"),
    ("singapore", "SG"),
    ("australia", "AU"),
];

/// Cities the owner's search profiles care about, used to fill the city field when the source gives a single combined location string.
const KNOWN_CITIES: &[(&str, &str, &str)] = &[
    ("karlsruhe", "Karlsruhe", "DE"),
    ("munich", "Munich", "DE"),
    ("muenchen", "Munich", "DE"),
    ("berlin", "Berlin", "DE"),
    ("hamburg", "Hamburg", "DE"),
    ("dresden", "Dresden", "DE"),
    ("potsdam", "Potsdam", "DE"),
    ("saarbruecken", "Saarbruecken", "DE"),
    ("darmstadt", "Darmstadt", "DE"),
    ("aachen", "Aachen", "DE"),
    ("erlangen", "Erlangen", "DE"),
    ("kaiserslautern", "Kaiserslautern", "DE"),
    ("bochum", "Bochum", "DE"),
    ("ilmenau", "Ilmenau", "DE"),
    ("garching", "Garching", "DE"),
    ("villach", "Villach", "AT"),
    ("eindhoven", "Eindhoven", "NL"),
    ("leuven", "Leuven", "BE"),
    ("bristol", "Bristol", "GB"),
    ("cambridge", "Cambridge", "GB"),
    ("stuttgart", "Stuttgart", "DE"),
    ("stockholm", "Stockholm", "SE"),
    ("gothenburg", "Gothenburg", "SE"),
    ("goeteborg", "Gothenburg", "SE"),
    ("lund", "Lund", "SE"),
    ("helsinki", "Helsinki", "FI"),
    ("tampere", "Tampere", "FI"),
    ("espoo", "Espoo", "FI"),
    ("oslo", "Oslo", "NO"),
    ("trondheim", "Trondheim", "NO"),
    ("dublin", "Dublin", "IE"),
    ("seattle", "Seattle", "US"),
    ("san francisco", "San Francisco", "US"),
    ("san jose", "San Jose", "US"),
    ("santa clara", "Santa Clara", "US"),
    ("austin", "Austin", "US"),
    ("vancouver", "Vancouver", "CA"),
    ("toronto", "Toronto", "CA"),
    ("zurich", "Zurich", "CH"),
    ("vienna", "Vienna", "AT"),
];

fn fold(value: &str) -> String {
    opportunity_domain::normalize_location(Some(value))
}

/// Best-effort country code for a source's free-text location. Returns `None` when the text does not name a country this table knows, which the rest of the system treats as unknown and never as a filter match.
/// ISO codes this tool recognises when a listing states the country as a bare code, as career sites commonly do ("Potsdam, DE, 14482").
const ISO_CODES: &[&str] = &[
    "DE", "SE", "FI", "NO", "DK", "NL", "IE", "GB", "US", "CA", "CH", "AT", "FR", "ES", "IT", "PL",
    "IL", "IN", "JP", "SG", "AU", "BE", "CZ", "PT", "RO", "HU", "UA", "TR", "KR", "TW", "CN", "BR",
    "MX", "NZ", "ZA", "AE",
];

pub fn country_code(location: &str) -> Option<String> {
    // Checked before folding: only an uppercase token counts, so the "in" of an English sentence is never read as India.
    for token in location.split(|c: char| !c.is_ascii_alphabetic()) {
        if token.len() == 2 && ISO_CODES.contains(&token) {
            return Some(token.to_owned());
        }
    }
    let folded = fold(location);
    let tokens: Vec<&str> = folded.split(' ').collect();
    for (name, code) in COUNTRIES {
        let needle: Vec<&str> = name.split(' ').collect();
        if needle.len() <= tokens.len()
            && tokens.windows(needle.len()).any(|w| w == needle.as_slice())
        {
            return Some((*code).to_owned());
        }
    }
    // A bare city name is common on Lever and Ashby boards, so fall back to the city table.
    city(location).map(|(_, code)| code)
}

/// Canonical city name and its country for a free-text location, when the text names a city this tool tracks.
pub fn city(location: &str) -> Option<(String, String)> {
    let folded = fold(location);
    let tokens: Vec<&str> = folded.split(' ').collect();
    for (needle_raw, display, code) in KNOWN_CITIES {
        let needle: Vec<&str> = needle_raw.split(' ').collect();
        if needle.len() <= tokens.len()
            && tokens.windows(needle.len()).any(|w| w == needle.as_slice())
        {
            return Some(((*display).to_owned(), (*code).to_owned()));
        }
    }
    None
}

/// Raw remote wording exactly as stated, or `None`. Interpreting it into an eligibility decision is session 3 work; storing the source's own words now keeps that decision honest.
pub fn remote_hint(location: &str) -> Option<String> {
    let folded = fold(location);
    if folded
        .split(' ')
        .any(|t| matches!(t, "remote" | "anywhere" | "distributed" | "hybrid"))
    {
        Some(location.trim().to_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn country_is_read_from_the_stated_location() {
        assert_eq!(country_code("Munich, Germany"), Some("DE".into()));
        assert_eq!(country_code("Stockholm, Sweden"), Some("SE".into()));
        assert_eq!(
            country_code("Seattle, WA, United States"),
            Some("US".into())
        );
    }

    #[test]
    fn a_bare_city_resolves_through_the_city_table() {
        assert_eq!(country_code("Karlsruhe"), Some("DE".into()));
        assert_eq!(
            city("Potsdam, DE, 14482"),
            Some(("Potsdam".into(), "DE".into()))
        );
        assert_eq!(city("Eindhoven"), Some(("Eindhoven".into(), "NL".into())));
        assert_eq!(city("Tampere"), Some(("Tampere".into(), "FI".into())));
    }

    #[test]
    fn a_bare_uppercase_iso_code_is_recognized_but_a_lowercase_word_is_not() {
        assert_eq!(country_code("Potsdam, DE, 14482"), Some("DE".into()));
        assert_eq!(country_code("Toronto, CAN"), Some("CA".into()));
        assert_eq!(country_code("Research in memory systems"), None);
        assert_eq!(country_code("Work in Austria"), Some("AT".into()));
    }

    #[test]
    fn an_unrecognized_location_stays_unknown_instead_of_being_guessed() {
        assert_eq!(country_code("Remote"), None);
        assert_eq!(country_code("Our lovely office"), None);
        assert_eq!(city("Nowhere Special"), None);
    }

    #[test]
    fn remote_wording_is_preserved_verbatim_not_interpreted() {
        assert_eq!(
            remote_hint("Remote - US only"),
            Some("Remote - US only".into())
        );
        assert_eq!(remote_hint("Munich, Germany"), None);
    }
}
