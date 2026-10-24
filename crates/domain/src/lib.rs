//! Domain types and rules. This crate performs no I/O so every rule here is unit testable offline.

pub mod academic;
pub mod application;
pub mod authorization;
pub mod classify;
pub mod completeness;
pub mod concepts;
pub mod dedupe;
pub mod matching;
pub mod normalize;
pub mod profile;
pub mod source;
pub mod types;

pub use academic::{academic_state, extract_deadline, extract_supervisor};
pub use application::{
    map_form, Fact, FieldKind, FormField, FormMapping, FormPlan, MappedField, OpenQuestion, Route,
};
pub use authorization::{authorize, Authorization, Policy, SubmissionRequest, Token};
pub use classify::{classify_kind, detect_advertised_state};
pub use completeness::{RunOutcome, MISSING_SWEEPS_BEFORE_CLOSE};
pub use dedupe::{content_hash, dedupe_key};
pub use matching::{
    evaluate, Candidate, Exclusion, ExclusionKind, ExclusionRule, Factor, MatchResult, Uncertainty,
    LEXICAL_MODEL,
};
pub use normalize::{
    decode_entities, normalize_location, normalize_organization, normalize_title, strip_html,
};
pub use profile::{Profile, RemotePolicy, SeniorityPolicy, Weights};
pub use source::{AcademicSelectors, SourceEndpoint, SourceHealth, SourceKind, SourceSupport};
pub use types::{
    AdvertisedState, FetchedPosting, OpportunityKind, RemoteEligibility, ScanResult, SourceRunStats,
};
