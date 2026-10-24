//! Deciding whether a submission is permitted.
//!
//! Two models, per the owner-approved amendment in `SCOPE_LOCK.md`. A standing policy lets routine applications on named sources go out without the owner seeing each one; a one-use token covers everything else. Both are off by default, both are revocable, and neither can be stretched: a policy that does not name a source does not cover it, and a token bound to one payload cannot be spent on another.
//!
//! Every refusal names its reason, because a submission that silently does not happen is as bad as one that silently does.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::application::Route;

/// A standing authorization as the decision function sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub name: String,
    pub enabled: bool,
    pub source_keys: Vec<String>,
    pub require_all_fields_mapped: bool,
    pub forbid_sensitive_questions: bool,
    pub forbid_free_text: bool,
    pub max_per_day: i32,
    pub max_total: Option<i32>,
    pub submissions_made: i32,
    /// Submissions made under *this* policy today. Counted per policy, because a daily limit is a property of the policy that set it: letting one policy's usage exhaust another's would silently block applications the owner authorized.
    pub submissions_today: i32,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// A one-use individual authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub payload_hash: String,
    pub destination: String,
    pub expires_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
}

/// What is being asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionRequest {
    pub source_key: String,
    pub destination: String,
    pub payload_hash: String,
    pub route: Route,
    pub open_questions: usize,
    pub has_sensitive_question: bool,
    pub has_free_text: bool,
    /// True when this application already has a submitted or uncertain attempt. Either blocks a further one.
    pub already_attempted: bool,
}

/// The verdict, with the reason spelled out either way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Authorization {
    /// Permitted by a standing policy, which is named so the audit trail can point at it.
    Policy {
        policy: String,
        reason: String,
    },
    /// Permitted by a one-use token bound to this application and payload.
    Token {
        reason: String,
    },
    Refused {
        reason: String,
    },
}

impl Authorization {
    pub fn is_permitted(&self) -> bool {
        !matches!(self, Self::Refused { .. })
    }

    pub fn reason(&self) -> &str {
        match self {
            Self::Policy { reason, .. } | Self::Token { reason } | Self::Refused { reason } => {
                reason
            }
        }
    }
}

/// Decide whether to submit.
///
/// The order matters. A duplicate is refused before anything else, because the worst outcome here is applying twice. Then a valid token, which is the owner saying yes to this exact submission. Then a policy, which has to pass every one of its own conditions.
pub fn authorize(
    request: &SubmissionRequest,
    token: Option<&Token>,
    policies: &[Policy],
    now: DateTime<Utc>,
) -> Authorization {
    if request.already_attempted {
        return Authorization::Refused {
            reason: "this application already has a submission attempt that was not resolved as failed; submitting again could apply twice".into(),
        };
    }

    if let Some(token) = token {
        return match () {
            _ if token.used_at.is_some() => Authorization::Refused {
                reason: "that authorization has already been used; a token is good for one submission".into(),
            },
            _ if token.expires_at <= now => Authorization::Refused {
                reason: "that authorization has expired".into(),
            },
            // Binding the payload is what stops a token granted for one set of answers being spent on another.
            _ if token.payload_hash != request.payload_hash => Authorization::Refused {
                reason: "the answers changed after you authorized this, so the authorization no longer applies".into(),
            },
            _ if token.destination != request.destination => Authorization::Refused {
                reason: "the destination changed after you authorized this".into(),
            },
            _ => Authorization::Token {
                reason: "you authorized this exact application and payload".into(),
            },
        };
    }

    // Without a token, only a standing policy can permit it, and only a routine application qualifies.
    if request.route != Route::Auto {
        return Authorization::Refused {
            reason: format!(
                "this application is routed as {} rather than automatic, so it needs you",
                request.route.as_str()
            ),
        };
    }

    let mut refusals = Vec::new();
    for policy in policies {
        match policy_permits(policy, request, now) {
            Ok(()) => {
                return Authorization::Policy {
                    policy: policy.name.clone(),
                    reason: format!("covered by your standing authorization '{}'", policy.name),
                }
            }
            Err(reason) => refusals.push(format!("'{}': {reason}", policy.name)),
        }
    }

    Authorization::Refused {
        reason: if refusals.is_empty() {
            "no standing authorization exists, and nothing authorized this application individually"
                .into()
        } else {
            format!(
                "no standing authorization covers this. {}",
                refusals.join("; ")
            )
        },
    }
}

fn policy_permits(
    policy: &Policy,
    request: &SubmissionRequest,
    now: DateTime<Utc>,
) -> Result<(), String> {
    if !policy.enabled {
        return Err("it is not enabled".into());
    }
    if policy.revoked_at.is_some_and(|at| at <= now) {
        return Err("it was revoked".into());
    }
    if policy.expires_at.is_some_and(|at| at <= now) {
        return Err("it has expired".into());
    }
    // An empty source list covers nothing. A policy has to name what it applies to, so one can never become accidentally global.
    if !policy
        .source_keys
        .iter()
        .any(|key| key == &request.source_key)
    {
        return Err(format!(
            "it does not cover the source '{}'",
            request.source_key
        ));
    }
    if policy.require_all_fields_mapped && request.open_questions > 0 {
        return Err(format!(
            "{} questions are unanswered and it requires every field to be mapped",
            request.open_questions
        ));
    }
    if policy.forbid_sensitive_questions && request.has_sensitive_question {
        return Err("the form asks a sensitive question".into());
    }
    if policy.forbid_free_text && request.has_free_text {
        return Err("the form asks for free text".into());
    }
    if policy.submissions_today >= policy.max_per_day {
        return Err(format!(
            "its daily limit of {} submissions is already used up",
            policy.max_per_day
        ));
    }
    if policy
        .max_total
        .is_some_and(|max| policy.submissions_made >= max)
    {
        return Err(format!(
            "its total limit of {} submissions is already used up",
            policy.max_total.unwrap_or_default()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    fn routine_request() -> SubmissionRequest {
        SubmissionRequest {
            source_key: "examplechip-greenhouse".into(),
            destination: "http://127.0.0.1:8099/apply".into(),
            payload_hash: "abc123".into(),
            route: Route::Auto,
            open_questions: 0,
            has_sensitive_question: false,
            has_free_text: false,
            already_attempted: false,
        }
    }

    fn enabled_policy() -> Policy {
        Policy {
            name: "routine greenhouse applications".into(),
            enabled: true,
            source_keys: vec!["examplechip-greenhouse".into()],
            require_all_fields_mapped: true,
            forbid_sensitive_questions: true,
            forbid_free_text: true,
            max_per_day: 5,
            max_total: Some(50),
            submissions_made: 0,
            submissions_today: 0,
            expires_at: Some(now() + Duration::days(30)),
            revoked_at: None,
        }
    }

    #[test]
    fn nothing_is_permitted_by_default() {
        let refused = authorize(&routine_request(), None, &[], now());
        assert!(!refused.is_permitted());
        assert!(refused
            .reason()
            .contains("no standing authorization exists"));
    }

    #[test]
    fn a_routine_application_on_a_covered_source_is_permitted_by_a_standing_policy() {
        let decision = authorize(&routine_request(), None, &[enabled_policy()], now());
        assert!(decision.is_permitted());
        match decision {
            Authorization::Policy { policy, .. } => {
                assert_eq!(policy, "routine greenhouse applications")
            }
            other => panic!("expected a policy decision, got {other:?}"),
        }
    }

    #[test]
    fn a_standard_thesis_application_follows_the_same_policy_as_any_other() {
        // Nothing in this decision can see the kind of opportunity. A thesis behind a routine form on a covered source is authorized exactly as an employment application would be.
        let thesis = SubmissionRequest {
            destination: "http://127.0.0.1:8099/thesis-apply".into(),
            ..routine_request()
        };
        assert!(authorize(&thesis, None, &[enabled_policy()], now()).is_permitted());
    }

    #[test]
    fn a_disabled_paused_revoked_or_expired_policy_permits_nothing() {
        let request = routine_request();
        for (label, policy) in [
            (
                "disabled",
                Policy {
                    enabled: false,
                    ..enabled_policy()
                },
            ),
            (
                "revoked",
                Policy {
                    revoked_at: Some(now() - Duration::minutes(1)),
                    ..enabled_policy()
                },
            ),
            (
                "expired",
                Policy {
                    expires_at: Some(now() - Duration::minutes(1)),
                    ..enabled_policy()
                },
            ),
        ] {
            let decision = authorize(&request, None, &[policy], now());
            assert!(
                !decision.is_permitted(),
                "a {label} policy must permit nothing"
            );
        }
    }

    #[test]
    fn a_policy_covers_only_the_sources_it_names_and_an_empty_list_covers_nothing() {
        let request = routine_request();
        let elsewhere = Policy {
            source_keys: vec!["other-source".into()],
            ..enabled_policy()
        };
        assert!(!authorize(&request, None, &[elsewhere], now()).is_permitted());

        let global_by_accident = Policy {
            source_keys: Vec::new(),
            ..enabled_policy()
        };
        let decision = authorize(&request, None, &[global_by_accident], now());
        assert!(!decision.is_permitted());
        assert!(decision.reason().contains("does not cover the source"));
    }

    #[test]
    fn a_policy_refuses_an_application_with_unanswered_or_sensitive_or_essay_questions() {
        let policy = enabled_policy();
        for request in [
            SubmissionRequest {
                open_questions: 1,
                ..routine_request()
            },
            SubmissionRequest {
                has_sensitive_question: true,
                ..routine_request()
            },
            SubmissionRequest {
                has_free_text: true,
                ..routine_request()
            },
        ] {
            assert!(
                !authorize(&request, None, std::slice::from_ref(&policy), now()).is_permitted()
            );
        }
    }

    #[test]
    fn one_policys_daily_usage_never_exhausts_another_policys_allowance() {
        // A daily limit belongs to the policy that set it. Counting submissions across every policy would let an unrelated policy's activity silently block applications the owner authorized under this one.
        let busy = Policy {
            name: "busy".into(),
            submissions_today: 5,
            ..enabled_policy()
        };
        let fresh = Policy {
            name: "fresh".into(),
            submissions_today: 0,
            ..enabled_policy()
        };
        let decision = authorize(&routine_request(), None, &[busy, fresh], now());
        assert!(decision.is_permitted(), "{}", decision.reason());
        match decision {
            Authorization::Policy { policy, .. } => assert_eq!(policy, "fresh"),
            other => panic!("expected the unexhausted policy, got {other:?}"),
        }
    }

    #[test]
    fn a_policy_stops_at_its_daily_and_total_limits() {
        let policy = enabled_policy();
        let at_daily = Policy {
            submissions_today: 5,
            ..policy.clone()
        };
        let decision = authorize(
            &routine_request(),
            None,
            std::slice::from_ref(&at_daily),
            now(),
        );
        assert!(!decision.is_permitted());
        assert!(decision.reason().contains("daily limit"));

        let exhausted = Policy {
            submissions_made: 50,
            ..policy
        };
        let decision = authorize(&routine_request(), None, &[exhausted], now());
        assert!(!decision.is_permitted());
        assert!(decision.reason().contains("total limit"));
    }

    #[test]
    fn a_policy_never_authorizes_an_application_that_is_not_routed_automatic() {
        for route in [Route::Review, Route::Curated] {
            let request = SubmissionRequest {
                route,
                ..routine_request()
            };
            let decision = authorize(&request, None, &[enabled_policy()], now());
            assert!(
                !decision.is_permitted(),
                "{route:?} must not be auto-submitted"
            );
            assert!(decision.reason().contains("needs you"));
        }
    }

    #[test]
    fn a_one_use_token_authorizes_exactly_its_own_application_and_payload() {
        let request = routine_request();
        let token = Token {
            payload_hash: "abc123".into(),
            destination: "http://127.0.0.1:8099/apply".into(),
            expires_at: now() + Duration::hours(1),
            used_at: None,
        };
        assert!(authorize(&request, Some(&token), &[], now()).is_permitted());

        // Spent, expired, or bound to different answers or a different destination: all refused.
        let spent = Token {
            used_at: Some(now()),
            ..token.clone()
        };
        assert!(!authorize(&request, Some(&spent), &[], now()).is_permitted());
        let expired = Token {
            expires_at: now() - Duration::minutes(1),
            ..token.clone()
        };
        assert!(!authorize(&request, Some(&expired), &[], now()).is_permitted());
        let changed_payload = Token {
            payload_hash: "different".into(),
            ..token.clone()
        };
        let decision = authorize(&request, Some(&changed_payload), &[], now());
        assert!(!decision.is_permitted());
        assert!(decision.reason().contains("answers changed"));
        let changed_destination = Token {
            destination: "http://127.0.0.1:8099/elsewhere".into(),
            ..token
        };
        assert!(!authorize(&request, Some(&changed_destination), &[], now()).is_permitted());
    }

    #[test]
    fn a_token_authorizes_a_review_routed_application_which_a_policy_never_would() {
        // This is the point of the individual model: the owner looked at it and said yes.
        let request = SubmissionRequest {
            route: Route::Review,
            open_questions: 0,
            ..routine_request()
        };
        let token = Token {
            payload_hash: "abc123".into(),
            destination: "http://127.0.0.1:8099/apply".into(),
            expires_at: now() + Duration::hours(1),
            used_at: None,
        };
        assert!(authorize(&request, Some(&token), &[], now()).is_permitted());
        assert!(!authorize(&request, None, &[enabled_policy()], now()).is_permitted());
    }

    #[test]
    fn an_application_that_already_has_an_attempt_is_refused_before_anything_else_is_considered() {
        let request = SubmissionRequest {
            already_attempted: true,
            ..routine_request()
        };
        let token = Token {
            payload_hash: "abc123".into(),
            destination: "http://127.0.0.1:8099/apply".into(),
            expires_at: now() + Duration::hours(1),
            used_at: None,
        };
        // Neither a valid token nor an enabled policy can override the duplicate guard.
        for decision in [
            authorize(&request, Some(&token), &[], now()),
            authorize(&request, None, &[enabled_policy()], now()),
        ] {
            assert!(!decision.is_permitted());
            assert!(decision.reason().contains("could apply twice"));
        }
    }
}
