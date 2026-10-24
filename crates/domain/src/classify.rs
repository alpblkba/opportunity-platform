use crate::normalize::{contains_phrase, normalize_title};
use crate::types::{AdvertisedState, OpportunityKind};

/// Phrase table for opportunity kinds. Order matters: the first match on the normalized title wins, so the most specific German and English phrases come first. Matching is phrase based on the normalized title rather than a regex over raw text, which is what keeps `FPGA Research Assistant` and `Electrical Design Engineer` from being filtered out by a software-only pattern.
const KIND_PHRASES: &[(&str, OpportunityKind)] = &[
    ("masterarbeit", OpportunityKind::MasterThesis),
    ("master arbeit", OpportunityKind::MasterThesis),
    ("abschlussarbeit", OpportunityKind::MasterThesis),
    ("master thesis", OpportunityKind::MasterThesis),
    ("masters thesis", OpportunityKind::MasterThesis),
    ("master s thesis", OpportunityKind::MasterThesis),
    ("diplomarbeit", OpportunityKind::MasterThesis),
    ("thesis", OpportunityKind::MasterThesis),
    ("doktorand", OpportunityKind::Phd),
    ("promotion", OpportunityKind::Phd),
    ("phd", OpportunityKind::Phd),
    ("ph d", OpportunityKind::Phd),
    ("doctoral", OpportunityKind::Phd),
    ("postdoc", OpportunityKind::Fellowship),
    ("fellowship", OpportunityKind::Fellowship),
    ("werkstudent", OpportunityKind::WorkingStudent),
    ("working student", OpportunityKind::WorkingStudent),
    ("hiwi", OpportunityKind::WorkingStudent),
    ("studentische hilfskraft", OpportunityKind::WorkingStudent),
    (
        "wissenschaftliche hilfskraft",
        OpportunityKind::ResearchAssistant,
    ),
    ("research assistant", OpportunityKind::ResearchAssistant),
    ("forschungsassistent", OpportunityKind::ResearchAssistant),
    ("praktikum", OpportunityKind::Internship),
    ("praktikant", OpportunityKind::Internship),
    ("internship", OpportunityKind::Internship),
    ("intern", OpportunityKind::Internship),
    ("co op", OpportunityKind::Internship),
    ("part time", OpportunityKind::PartTime),
    ("teilzeit", OpportunityKind::PartTime),
];

/// Classify a posting. Nothing is ever dropped: a title that matches no phrase is ordinary employment, not a rejection.
pub fn classify_kind(title: &str, employment_type_raw: Option<&str>) -> OpportunityKind {
    let normalized = normalize_title(title);
    for (phrase, kind) in KIND_PHRASES {
        if contains_phrase(&normalized, phrase) {
            return *kind;
        }
    }
    if let Some(raw) = employment_type_raw {
        let normalized_raw = normalize_title(raw);
        for (phrase, kind) in KIND_PHRASES {
            if contains_phrase(&normalized_raw, phrase) {
                return *kind;
            }
        }
    }
    OpportunityKind::Employment
}

/// Whether the record is an advertised vacancy or only a topic or a supervisor contact. ATS postings are advertised by construction; academic pages in later sessions supply the other states, so this function keeps the distinction explicit from the start rather than letting `open_topic` silently become a funded role.
pub fn detect_advertised_state(kind: OpportunityKind, from_ats: bool) -> AdvertisedState {
    if from_ats {
        AdvertisedState::Advertised
    } else if matches!(kind, OpportunityKind::MasterThesis) {
        AdvertisedState::OpenTopic
    } else {
        AdvertisedState::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn german_thesis_titles_are_recognized() {
        assert_eq!(
            classify_kind("Masterarbeit: DRAM-Zuverlässigkeit", None),
            OpportunityKind::MasterThesis
        );
        assert_eq!(
            classify_kind("Abschlussarbeit Speichersysteme", None),
            OpportunityKind::MasterThesis
        );
        assert_eq!(
            classify_kind("Werkstudent Firmware (m/w/d)", None),
            OpportunityKind::WorkingStudent
        );
    }

    #[test]
    fn hardware_titles_that_a_software_only_filter_would_drop_are_kept() {
        assert_eq!(
            classify_kind("FPGA Research Assistant", None),
            OpportunityKind::ResearchAssistant
        );
        assert_eq!(
            classify_kind("Electrical Design Engineer", None),
            OpportunityKind::Employment
        );
        assert_eq!(
            classify_kind("Field Application Engineer", None),
            OpportunityKind::Employment
        );
    }

    #[test]
    fn substring_collisions_do_not_misclassify() {
        assert_eq!(
            classify_kind("International Sales Lead", None),
            OpportunityKind::Employment
        );
        assert_eq!(
            classify_kind("Internal Tools Engineer", None),
            OpportunityKind::Employment
        );
        assert_eq!(
            classify_kind("Software Engineering Intern", None),
            OpportunityKind::Internship
        );
    }

    #[test]
    fn employment_type_field_is_consulted_when_the_title_is_plain() {
        assert_eq!(
            classify_kind("Platform Engineer", Some("Internship")),
            OpportunityKind::Internship
        );
        assert_eq!(
            classify_kind("Platform Engineer", None),
            OpportunityKind::Employment
        );
    }

    #[test]
    fn an_open_topic_is_never_reported_as_an_advertised_vacancy() {
        assert_eq!(
            detect_advertised_state(OpportunityKind::MasterThesis, false),
            AdvertisedState::OpenTopic
        );
        assert_eq!(
            detect_advertised_state(OpportunityKind::MasterThesis, true),
            AdvertisedState::Advertised
        );
    }
}
