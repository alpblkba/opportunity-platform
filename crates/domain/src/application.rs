//! Application routing and form-field mapping.
//!
//! Routing decides how an application gets completed: automatically, by asking the owner some questions, or by hand. The decision is made from the form in front of us and the facts we hold, never from the kind of opportunity. A thesis application behind a standard form is as automatable as any other; an employment application asking why you want the job is not.
//!
//! Nothing here invents a fact. A field that cannot be filled from an approved fact becomes a question for the owner, and that is the whole mechanism by which employment history, dates, education and work authorization stay truthful.

use serde::{Deserialize, Serialize};

use crate::normalize::normalize_location;

/// How an application should be completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// Every required field maps to an approved fact, the form is routine, and an authorization permits it.
    Auto,
    /// Something needs the owner: a missing answer, a sensitive question, a control this tool will not operate.
    Review,
    /// The form asks for substantial personalized writing. Preparing it is useful; submitting it autonomously is not.
    Curated,
}

impl Route {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Review => "review",
            Self::Curated => "curated",
        }
    }
}

/// What kind of control a form field is, as the worker observed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Text,
    Email,
    Phone,
    Url,
    Select,
    Checkbox,
    Radio,
    /// A multi-line box. Short ones are routine; long ones are where essays live.
    // Renamed because the browser worker reports the HTML tag name; the snake_case derive would otherwise expect `text_area` and fail to parse every multi-line field.
    #[serde(rename = "textarea")]
    TextArea,
    File,
    /// Something this tool does not know how to operate safely.
    Unsupported,
}

/// One field the worker found on a form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormField {
    pub name: String,
    pub label: String,
    pub kind: FieldKind,
    pub required: bool,
    #[serde(default)]
    pub options: Vec<String>,
    /// Present when the form declared a maximum length, which helps tell a short answer box from an essay box.
    #[serde(default)]
    pub max_length: Option<i32>,
}

/// What the worker found when it inspected a form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormPlan {
    pub url: String,
    pub fields: Vec<FormField>,
    #[serde(default = "one")]
    pub steps: i32,
    /// A challenge this tool will not attempt. Its presence sends the application to a human, every time.
    #[serde(default)]
    pub has_captcha: bool,
    #[serde(default)]
    pub requires_login: bool,
    #[serde(default)]
    pub submit_selector: Option<String>,
}

fn one() -> i32 {
    1
}

/// An approved claim about the applicant, as the matcher sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact {
    pub key: String,
    pub value: String,
    pub sensitive: bool,
}

/// One field paired with what will go in it, and where that came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MappedField {
    pub field: String,
    pub label: String,
    pub value: String,
    /// The approved fact or answer this value came from. Every filled value has one.
    pub source: String,
}

/// A field that could not be filled from anything the owner approved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenQuestion {
    pub field: String,
    pub label: String,
    pub kind: FieldKind,
    pub required: bool,
    pub sensitive: bool,
    /// A suggestion assembled from approved facts, or nothing. Never a guess at a fact the owner has not confirmed.
    pub draft: Option<String>,
    pub reason: String,
}

/// The complete result of mapping a form against the facts on hand.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormMapping {
    pub mapped: Vec<MappedField>,
    pub open: Vec<OpenQuestion>,
    pub route: Route,
    pub reason: String,
}

/// Question wording that asks for substantive personal writing rather than a fact. These are the fields that make an application worth doing by hand.
const ESSAY_PHRASES: &[&str] = &[
    "why do you want",
    "why are you interested",
    "why this role",
    "why our company",
    "tell us about",
    "describe a time",
    "describe a project",
    "cover letter",
    "motivation",
    "motivationsschreiben",
    "anschreiben",
    "research statement",
    "research interests",
    "research proposal",
    "statement of purpose",
    "what interests you",
    "in your own words",
    "personal statement",
];

/// Wording that marks a question as one no system should answer on someone's behalf without their explicit confirmation of that exact value.
const SENSITIVE_PHRASES: &[&str] = &[
    "citizenship",
    "staatsangehoerigkeit",
    "work authorization",
    "right to work",
    "visa",
    "sponsorship",
    "disability",
    "schwerbehinderung",
    "veteran",
    "criminal",
    "conviction",
    "security clearance",
    "clearance",
    "ethnicity",
    "race",
    "gender",
    "geschlecht",
    "date of birth",
    "geburtsdatum",
    "salary expectation",
    "gehaltsvorstellung",
    "notice period",
];

/// A free-text box longer than this is treated as an essay rather than a short answer.
const ESSAY_MIN_LENGTH: i32 = 400;

fn matches_any(label: &str, phrases: &[&str]) -> Option<String> {
    let folded = normalize_location(Some(label));
    phrases
        .iter()
        .find(|phrase| {
            let needle: Vec<&str> = phrase.split(' ').collect();
            let hay: Vec<&str> = folded.split(' ').collect();
            needle.len() <= hay.len() && hay.windows(needle.len()).any(|w| w == needle.as_slice())
        })
        .map(|phrase| (*phrase).to_owned())
}

/// Whether a field asks for substantive personal writing.
pub fn is_essay(field: &FormField) -> bool {
    if matches_any(&field.label, ESSAY_PHRASES).is_some() {
        return true;
    }
    // A long free-text box with no stated limit is an essay box whatever it is labelled.
    field.kind == FieldKind::TextArea
        && field
            .max_length
            .map(|m| m >= ESSAY_MIN_LENGTH)
            .unwrap_or(true)
}

/// Whether a field asks something that must come from the owner directly.
pub fn is_sensitive(field: &FormField) -> bool {
    matches_any(&field.label, SENSITIVE_PHRASES).is_some()
}

/// Map a form against approved facts and decide how the application should be completed.
///
/// The routing rules in order: a challenge or an unsupported control sends it to a human; a substantive writing question makes it curated; a missing or sensitive answer makes it a review; everything else is automatable as far as the form is concerned, which an authorization then has to permit separately.
pub fn map_form(
    plan: &FormPlan,
    facts: &[Fact],
    answers: &[(String, String, bool)],
) -> FormMapping {
    let mut mapped = Vec::new();
    let mut open = Vec::new();
    let mut essay_fields = Vec::new();

    for field in &plan.fields {
        if field.kind == FieldKind::Unsupported {
            open.push(OpenQuestion {
                field: field.name.clone(),
                label: field.label.clone(),
                kind: field.kind,
                required: field.required,
                sensitive: false,
                draft: None,
                reason: "this tool does not know how to operate this control safely".into(),
            });
            continue;
        }
        if field.kind == FieldKind::File {
            // A document is attached by the worker from the chosen approved file, not typed into a field.
            mapped.push(MappedField {
                field: field.name.clone(),
                label: field.label.clone(),
                value: "<approved document>".into(),
                source: "selected document".into(),
            });
            continue;
        }

        let sensitive = is_sensitive(field);
        let essay = is_essay(field);
        if essay {
            essay_fields.push(field.label.clone());
        }

        // A previously approved reusable answer wins, then an approved fact. Both are things the owner confirmed.
        let from_answer = answers
            .iter()
            .find(|(key, _, approved)| *approved && key_matches(key, &field.name, &field.label));
        let from_fact = facts
            .iter()
            .find(|fact| key_matches(&fact.key, &field.name, &field.label));

        match (from_answer, from_fact) {
            _ if essay => open.push(OpenQuestion {
                field: field.name.clone(),
                label: field.label.clone(),
                kind: field.kind,
                required: field.required,
                sensitive,
                draft: None,
                reason: "this asks for personal writing, which is yours to do".into(),
            }),
            (Some((key, value, _)), _) => mapped.push(MappedField {
                field: field.name.clone(),
                label: field.label.clone(),
                value: value.clone(),
                source: format!("your saved answer '{key}'"),
            }),
            (None, Some(fact)) if !fact.sensitive || !sensitive => mapped.push(MappedField {
                field: field.name.clone(),
                label: field.label.clone(),
                value: fact.value.clone(),
                source: format!("your approved fact '{}'", fact.key),
            }),
            (None, Some(fact)) => open.push(OpenQuestion {
                field: field.name.clone(),
                label: field.label.clone(),
                kind: field.kind,
                required: field.required,
                sensitive: true,
                // The owner has approved a value, so it is offered as a draft, but a sensitive question is still confirmed each time.
                draft: Some(fact.value.clone()),
                reason: "this is a sensitive question, so it is confirmed rather than filled automatically".into(),
            }),
            (None, None) => open.push(OpenQuestion {
                field: field.name.clone(),
                label: field.label.clone(),
                kind: field.kind,
                required: field.required,
                sensitive,
                draft: None,
                reason: if sensitive {
                    "this is a sensitive question and you have not recorded an answer".into()
                } else {
                    "no approved fact answers this, and nothing was guessed".into()
                },
            }),
        }
    }

    let unresolved_required = open.iter().filter(|q| q.required).count();
    let (route, reason) = if plan.has_captcha {
        (
            Route::Review,
            "the form presents a challenge this tool will not attempt".into(),
        )
    } else if plan.requires_login {
        (
            Route::Review,
            "the form requires signing in, which you do yourself".into(),
        )
    } else if !essay_fields.is_empty() {
        (
            Route::Curated,
            format!(
                "this form asks for personal writing: {}",
                essay_fields.join(", ")
            ),
        )
    } else if unresolved_required > 0 {
        (
            Route::Review,
            format!("{unresolved_required} required questions need an answer from you"),
        )
    } else if open.iter().any(|q| q.sensitive) {
        (
            Route::Review,
            "a sensitive question needs your confirmation".into(),
        )
    } else {
        (
            Route::Auto,
            format!(
                "every one of the {} fields maps to something you approved",
                mapped.len()
            ),
        )
    };

    FormMapping {
        mapped,
        open,
        route,
        reason,
    }
}

/// Whether a stored key answers a form field, comparing against both the field's machine name and its visible label.
fn key_matches(key: &str, field_name: &str, label: &str) -> bool {
    let key_folded = normalize_location(Some(key));
    if key_folded.is_empty() {
        return false;
    }
    let name_folded = normalize_location(Some(field_name));
    let label_folded = normalize_location(Some(label));
    key_folded == name_folded || key_folded == label_folded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, label: &str, kind: FieldKind, required: bool) -> FormField {
        FormField {
            name: name.into(),
            label: label.into(),
            kind,
            required,
            options: Vec::new(),
            max_length: None,
        }
    }

    fn short_text(name: &str, label: &str) -> FormField {
        FormField {
            max_length: Some(120),
            ..field(name, label, FieldKind::Text, true)
        }
    }

    fn standard_facts() -> Vec<Fact> {
        vec![
            Fact {
                key: "full name".into(),
                value: "A. Example".into(),
                sensitive: false,
            },
            Fact {
                key: "email".into(),
                value: "a@example.test".into(),
                sensitive: false,
            },
            Fact {
                key: "phone".into(),
                value: "+49 000 000".into(),
                sensitive: false,
            },
        ]
    }

    fn routine_plan(url: &str) -> FormPlan {
        FormPlan {
            url: url.into(),
            fields: vec![
                short_text("full name", "Full name"),
                short_text("email", "Email"),
                short_text("phone", "Phone"),
                field("cv", "Upload your CV", FieldKind::File, true),
            ],
            steps: 1,
            has_captcha: false,
            requires_login: false,
            submit_selector: Some("#submit".into()),
        }
    }

    #[test]
    fn a_routine_form_whose_fields_all_map_is_automatable() {
        let mapping = map_form(
            &routine_plan("http://127.0.0.1:8099/apply"),
            &standard_facts(),
            &[],
        );
        assert_eq!(mapping.route, Route::Auto);
        assert!(mapping.open.is_empty(), "{:?}", mapping.open);
        assert_eq!(mapping.mapped.len(), 4);
        // Every filled value names where it came from, which is what makes a submission auditable.
        assert!(mapping.mapped.iter().all(|m| !m.source.is_empty()));
    }

    #[test]
    fn routing_is_decided_by_the_form_and_never_by_the_kind_of_opportunity() {
        // The same routine form is automatable whether the opportunity behind it is a thesis or a job. Nothing in this function can see the opportunity at all.
        let thesis_form = routine_plan("http://127.0.0.1:8099/thesis-apply");
        let job_form = routine_plan("http://127.0.0.1:8099/job-apply");
        assert_eq!(
            map_form(&thesis_form, &standard_facts(), &[]).route,
            Route::Auto
        );
        assert_eq!(
            map_form(&job_form, &standard_facts(), &[]).route,
            Route::Auto
        );

        // And a job form carrying an essay is curated, for exactly the same reason a thesis form would be.
        let mut essay_job = routine_plan("http://127.0.0.1:8099/job-apply");
        essay_job.fields.push(field(
            "motivation",
            "Why do you want this role?",
            FieldKind::TextArea,
            true,
        ));
        assert_eq!(
            map_form(&essay_job, &standard_facts(), &[]).route,
            Route::Curated
        );
    }

    #[test]
    fn a_question_with_no_approved_answer_becomes_a_review_task_and_is_never_guessed() {
        let mut plan = routine_plan("http://127.0.0.1:8099/apply");
        plan.fields.push(short_text(
            "years of experience",
            "Years of relevant experience",
        ));
        let mapping = map_form(&plan, &standard_facts(), &[]);

        assert_eq!(mapping.route, Route::Review);
        let question = mapping
            .open
            .iter()
            .find(|q| q.field == "years of experience")
            .unwrap();
        assert!(
            question.draft.is_none(),
            "an unknown fact must not be drafted"
        );
        assert!(question.reason.contains("nothing was guessed"));
        assert!(!mapping
            .mapped
            .iter()
            .any(|m| m.field == "years of experience"));
    }

    #[test]
    fn a_sensitive_question_is_confirmed_rather_than_filled_even_when_an_approved_value_exists() {
        let mut plan = routine_plan("http://127.0.0.1:8099/apply");
        plan.fields.push(short_text(
            "work authorization",
            "Do you have work authorization?",
        ));
        let mut facts = standard_facts();
        facts.push(Fact {
            key: "work authorization".into(),
            value: "Yes, EU".into(),
            sensitive: true,
        });

        let mapping = map_form(&plan, &facts, &[]);
        assert_eq!(mapping.route, Route::Review);
        let question = mapping
            .open
            .iter()
            .find(|q| q.field == "work authorization")
            .unwrap();
        assert!(question.sensitive);
        // The approved value is offered as a draft so answering is quick, but it is not submitted unconfirmed.
        assert_eq!(question.draft.as_deref(), Some("Yes, EU"));
        assert!(!mapping
            .mapped
            .iter()
            .any(|m| m.field == "work authorization"));
    }

    #[test]
    fn a_substantive_research_question_makes_the_application_curated() {
        let mut plan = routine_plan("http://127.0.0.1:8099/thesis-apply");
        plan.fields.push(field(
            "statement",
            "Describe your research interests",
            FieldKind::TextArea,
            true,
        ));
        let mapping = map_form(&plan, &standard_facts(), &[]);
        assert_eq!(mapping.route, Route::Curated);
        assert!(mapping.reason.contains("personal writing"));
        let question = mapping
            .open
            .iter()
            .find(|q| q.field == "statement")
            .unwrap();
        assert!(
            question.draft.is_none(),
            "a research statement is never drafted from facts"
        );
    }

    #[test]
    fn a_challenge_or_a_login_always_sends_the_application_to_a_human() {
        let mut captcha = routine_plan("http://127.0.0.1:8099/apply");
        captcha.has_captcha = true;
        let mapping = map_form(&captcha, &standard_facts(), &[]);
        assert_eq!(mapping.route, Route::Review);
        assert!(mapping.reason.contains("challenge"));

        let mut login = routine_plan("http://127.0.0.1:8099/apply");
        login.requires_login = true;
        assert_eq!(
            map_form(&login, &standard_facts(), &[]).route,
            Route::Review
        );
    }

    #[test]
    fn an_unsupported_control_is_reported_rather_than_operated() {
        let mut plan = routine_plan("http://127.0.0.1:8099/apply");
        plan.fields.push(field(
            "signature",
            "Draw your signature",
            FieldKind::Unsupported,
            true,
        ));
        let mapping = map_form(&plan, &standard_facts(), &[]);
        assert_eq!(mapping.route, Route::Review);
        let question = mapping
            .open
            .iter()
            .find(|q| q.field == "signature")
            .unwrap();
        assert!(question.reason.contains("does not know how to operate"));
    }

    #[test]
    fn an_approved_saved_answer_fills_a_question_that_no_fact_covers() {
        let mut plan = routine_plan("http://127.0.0.1:8099/apply");
        plan.fields
            .push(short_text("how did you hear", "How did you hear about us?"));
        let answers = vec![(
            "how did you hear".to_string(),
            "Company website".to_string(),
            true,
        )];
        let mapping = map_form(&plan, &standard_facts(), &answers);
        assert_eq!(mapping.route, Route::Auto);
        let filled = mapping
            .mapped
            .iter()
            .find(|m| m.field == "how did you hear")
            .unwrap();
        assert_eq!(filled.value, "Company website");
        assert!(filled.source.contains("saved answer"));
    }

    #[test]
    fn an_unapproved_saved_answer_is_not_used() {
        let mut plan = routine_plan("http://127.0.0.1:8099/apply");
        plan.fields
            .push(short_text("how did you hear", "How did you hear about us?"));
        let answers = vec![(
            "how did you hear".to_string(),
            "Company website".to_string(),
            false,
        )];
        let mapping = map_form(&plan, &standard_facts(), &answers);
        assert_eq!(mapping.route, Route::Review);
    }

    #[test]
    fn a_short_labelled_text_box_is_not_mistaken_for_an_essay() {
        let short = FormField {
            max_length: Some(80),
            ..field("city", "City", FieldKind::TextArea, true)
        };
        assert!(!is_essay(&short));
        let unbounded = field("notes", "Notes", FieldKind::TextArea, false);
        assert!(
            is_essay(&unbounded),
            "a free-text box with no stated limit is treated as an essay box"
        );
    }

    #[test]
    fn field_kinds_use_exactly_the_names_the_browser_worker_sends() {
        // The worker reports the HTML tag or input type verbatim. Pinning the wire names here means a rename on either side fails this test rather than silently breaking every form inspection.
        for (kind, wire) in [
            (FieldKind::Text, "text"),
            (FieldKind::Email, "email"),
            (FieldKind::Phone, "phone"),
            (FieldKind::Url, "url"),
            (FieldKind::Select, "select"),
            (FieldKind::Checkbox, "checkbox"),
            (FieldKind::Radio, "radio"),
            (FieldKind::TextArea, "textarea"),
            (FieldKind::File, "file"),
            (FieldKind::Unsupported, "unsupported"),
        ] {
            assert_eq!(serde_json::to_string(&kind).unwrap(), format!("\"{wire}\""));
            assert_eq!(
                serde_json::from_str::<FieldKind>(&format!("\"{wire}\"")).unwrap(),
                kind
            );
        }
    }

    #[test]
    fn sensitive_wording_is_recognized_in_english_and_german() {
        assert!(is_sensitive(&short_text(
            "x",
            "Do you require visa sponsorship?"
        )));
        assert!(is_sensitive(&short_text("x", "Staatsangehoerigkeit")));
        assert!(is_sensitive(&short_text("x", "Gehaltsvorstellung")));
        assert!(!is_sensitive(&short_text("x", "Full name")));
    }
}
