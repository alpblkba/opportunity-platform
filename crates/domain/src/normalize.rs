/// Fold a string to a comparison form: lowercase, accent-insensitive for the German umlauts we actually meet, punctuation collapsed to single spaces.
fn fold(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut pending_space = false;
    for ch in value.chars() {
        let umlaut: &str = match ch {
            'Ä' | 'ä' => "ae",
            'Ö' | 'ö' => "oe",
            'Ü' | 'ü' => "ue",
            'ß' => "ss",
            _ => "",
        };
        if umlaut.is_empty() && !ch.is_alphanumeric() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        if umlaut.is_empty() {
            out.extend(ch.to_lowercase());
        } else {
            out.push_str(umlaut);
        }
    }
    out
}

/// Legal-form and punctuation noise that must not make two spellings of the same employer look like different employers.
const ORG_NOISE: &[&str] = &[
    "inc",
    "incorporated",
    "llc",
    "ltd",
    "limited",
    "plc",
    "corp",
    "corporation",
    "co",
    "gmbh",
    "mbh",
    "ag",
    "kg",
    "ug",
    "se",
    "ab",
    "oy",
    "oyj",
    "as",
    "asa",
    "bv",
    "nv",
    "sa",
    "sas",
    "srl",
    "spa",
    "group",
    "holding",
    "holdings",
    "the",
];

/// Canonical employer identity used for exact alias matching and duplicate grouping. Deliberately exact after folding rather than substring based, so `Core Systems` never matches `Hardcore Systems`.
pub fn normalize_organization(name: &str) -> String {
    let folded = fold(name);
    let kept: Vec<&str> = folded
        .split_whitespace()
        .filter(|token| !ORG_NOISE.contains(token))
        .collect();
    if kept.is_empty() {
        folded
    } else {
        kept.join(" ")
    }
}

/// Requisition identifiers, location suffixes and job-board decoration make the same role look like two roles across sources, so they are stripped before comparison.
pub fn normalize_title(title: &str) -> String {
    // Parenthesised and bracketed groups are dropped, but the text after them is kept. Truncating at the first bracket instead would merge "PhD Candidate (m/f/x) in Lab A" with "PhD Candidate (m/f/x) in Lab B", which are two different positions.
    let mut stripped = String::with_capacity(title.len());
    let mut depth = 0usize;
    for ch in title.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => stripped.push(ch),
            _ => {}
        }
    }
    let folded = fold(&stripped);
    let kept: Vec<&str> = folded
        .split_whitespace()
        .filter(|token| !token.chars().all(|c| c.is_ascii_digit()))
        .filter(|token| !matches!(*token, "m" | "f" | "d" | "w" | "x" | "all" | "genders"))
        .collect();
    kept.join(" ")
}

/// Normalized location token used in the duplicate key. An empty result means the source did not state a location, which stays distinct from any stated location.
pub fn normalize_location(location: Option<&str>) -> String {
    location.map(fold).unwrap_or_default()
}

/// Whole-phrase containment on an already folded, space-separated string. Substring matching would let `intern` fire on `international`, so both ends of a hit must sit on a word boundary.
///
/// Scanning rather than splitting matters here: matching runs this tens of times against every opportunity's whole description, and building a word vector per call was most of the cost of ranking the corpus.
pub(crate) fn contains_phrase(haystack: &str, phrase: &str) -> bool {
    if phrase.is_empty() || phrase.len() > haystack.len() {
        return false;
    }
    let bytes = haystack.as_bytes();
    haystack.match_indices(phrase).any(|(start, _)| {
        let end = start + phrase.len();
        (start == 0 || bytes[start - 1] == b' ') && (end == bytes.len() || bytes[end] == b' ')
    })
}

/// Decode the handful of HTML entities that ATS payloads actually contain. Greenhouse returns its description with the markup itself entity-escaped, so this has to run before tag stripping for that source.
pub fn decode_entities(value: &str) -> String {
    value
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&rsquo;", "'")
        .replace("&uuml;", "\u{fc}")
        .replace("&auml;", "\u{e4}")
        .replace("&ouml;", "\u{f6}")
        .replace("&szlig;", "\u{df}")
        .replace("&amp;", "&")
}

/// Remove HTML tags and decode entities, so lexical search and length checks operate on readable text. This is not a sanitizer for rendering; the frontend never renders this output as HTML.
pub fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode_entities(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn organization_spellings_fold_to_one_identity() {
        assert_eq!(
            normalize_organization("Example Systems GmbH"),
            "example systems"
        );
        assert_eq!(
            normalize_organization("EXAMPLE  SYSTEMS, Inc."),
            "example systems"
        );
        assert_eq!(
            normalize_organization("Münchener Halbleiter AG"),
            "muenchener halbleiter"
        );
    }

    #[test]
    fn exact_identity_does_not_collapse_an_unrelated_substring_match() {
        assert_ne!(
            normalize_organization("Core Systems"),
            normalize_organization("Hardcore Systems")
        );
        assert_ne!(
            normalize_organization("Apple"),
            normalize_organization("Applebee")
        );
    }

    #[test]
    fn title_normalization_drops_requisition_and_gender_decoration() {
        assert_eq!(
            normalize_title("Firmware Engineer (m/w/d)"),
            "firmware engineer"
        );
        assert_eq!(
            normalize_title("Firmware Engineer [R-12345]"),
            "firmware engineer"
        );
        assert_eq!(
            normalize_title("Masterarbeit: Speichersysteme"),
            "masterarbeit speichersysteme"
        );
    }

    #[test]
    fn two_roles_that_differ_only_after_a_gender_marker_stay_distinct() {
        let a = normalize_title(
            "PhD Candidate (m/f/x) in the Cyber Threat Intelligence Lab - \"KriKom\"",
        );
        let b = normalize_title(
            "PhD Candidate (m/f/x) in the Cyber Threat Intelligence Lab - \"ReDiKom\"",
        );
        assert_ne!(
            a, b,
            "two different positions must not normalize to the same title"
        );
        assert!(a.starts_with("phd candidate in the cyber threat intelligence lab"));
    }

    #[test]
    fn html_stripping_yields_searchable_text_without_tags() {
        let html =
            "<p>Work on <strong>DRAM</strong>&nbsp;reliability</p><ul><li>C&amp;C++</li></ul>";
        assert_eq!(strip_html(html), "Work on DRAM reliability C&C++");
    }

    #[test]
    fn html_stripping_does_not_merge_adjacent_block_text() {
        assert_eq!(
            strip_html("<li>Munich</li><li>Berlin</li>"),
            "Munich Berlin"
        );
    }

    #[test]
    fn a_phrase_matches_only_on_whole_words() {
        assert!(contains_phrase("senior intern engineer", "intern"));
        assert!(!contains_phrase("international engineer", "intern"));
        assert!(!contains_phrase("winterns", "intern"));
        // Multi-word phrases have to sit on boundaries at both ends too.
        assert!(contains_phrase("board bring up engineer", "bring up"));
        assert!(!contains_phrase("bringing upstream", "bring up"));
        // The ends of the string are boundaries.
        assert!(contains_phrase("firmware", "firmware"));
        assert!(contains_phrase("rtos firmware", "firmware"));
        assert!(contains_phrase("firmware rtos", "firmware"));
        assert!(!contains_phrase("", "firmware"));
        assert!(!contains_phrase("firmware", ""));
    }
}
