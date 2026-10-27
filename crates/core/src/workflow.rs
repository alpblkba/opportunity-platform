//! The application workflow: prepare, fill, submit.
//!
//! This is where routing, facts, authorization, the browser worker and the state machine meet. Each step is small and writes what it did, so an application's history explains itself. The ordering is the safety property: authorization is decided from current database state immediately before the attempt is claimed, and the attempt is claimed before the browser is touched.

use opportunity_domain::{map_form, FormMapping, FormPlan, MappedField, Route};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::applications::{self, AppError};
use crate::db::Db;
use crate::scheduler::WorkLease;
use crate::worker;

/// How long a browser run waits for a source sweep to finish before giving up.
///
/// A full sweep of every source measured 16 seconds, so a minute is generous. Waiting is better than failing, and failing is better than hanging: somebody is holding a phone while this happens.
const BROWSER_PATIENCE: std::time::Duration = std::time::Duration::from_secs(60);

/// Claim the machine for one browser run.
///
/// Chromium and a source sweep do not fit together on a small host, and the sweep is the one that starts by itself. Everything that drives the browser comes through here.
pub async fn claim_browser_slot(pool: &Db) -> Result<WorkLease, AppError> {
    WorkLease::acquire_within(pool, BROWSER_PATIENCE)
        .await?
        .ok_or_else(|| {
            AppError::Invalid(
                "a source refresh is still running, so the browser cannot have the machine yet. Try again in a moment."
                    .into(),
            )
        })
}

#[derive(Debug, Clone, Serialize)]
pub struct PrepareResult {
    pub application_id: Uuid,
    pub route: String,
    pub reason: String,
    pub mapped: usize,
    pub open_questions: usize,
    pub plan: FormPlan,
    pub mapping: FormMapping,
}

/// A worker failure as the owner should read it. A browser that is absent by design is a handoff, not a failed attempt, so it is not dressed up as one.
pub fn worker_message(error: &worker::WorkerError) -> String {
    match error {
        worker::WorkerError::Unavailable(message) => message.clone(),
        other => format!("could not inspect the form: {other}"),
    }
}

/// Fold the answers the owner has already given in the review queue into a fresh mapping.
///
/// A question the owner has answered is not open any more: its value is filled like any other, and a review that has been worked through becomes automatable. Everything that maps a form goes through here, so the interface never keeps asking for an answer it already holds and what the owner is shown is what will be submitted. Curated stays curated: personal writing is the owner's to send.
pub async fn with_answers(
    pool: &Db,
    application_id: Uuid,
    mut mapping: FormMapping,
) -> Result<FormMapping, AppError> {
    let answered = applications::answered_tasks(pool, application_id).await?;
    if answered.is_empty() {
        return Ok(mapping);
    }

    let mut still_open = Vec::new();
    for question in std::mem::take(&mut mapping.open) {
        match answered.iter().find(|(field, _)| *field == question.field) {
            Some((_, answer)) if question.kind != opportunity_domain::FieldKind::Unsupported => {
                mapping.mapped.push(MappedField {
                    field: question.field,
                    label: question.label,
                    value: answer.clone(),
                    source: "your answer in the review queue".into(),
                })
            }
            _ => still_open.push(question),
        }
    }
    mapping.open = still_open;

    if (mapping.route == Route::Review || mapping.route == Route::Curated)
        && mapping.open.is_empty()
        && !mapping.reason.contains("challenge")
        && !mapping.reason.contains("signing in")
    {
        mapping.route = Route::Auto;
        mapping.reason = format!(
            "every one of the {} fields maps to something you approved or answered",
            mapping.mapped.len()
        );
    }
    Ok(mapping)
}

/// Inspect the form behind an application, map it against approved facts, and record what remains unanswered.
///
/// The route is decided here and stored, but it grants nothing: submitting still has to pass authorization separately.
pub async fn prepare(
    pool: &Db,
    application_id: Uuid,
    url: &str,
) -> Result<PrepareResult, AppError> {
    let slot = claim_browser_slot(pool).await?;
    let was = applications::get_application(pool, application_id)
        .await?
        .status;
    applications::set_status(
        pool,
        application_id,
        "preparing",
        serde_json::json!({ "url": url }),
    )
    .await?;

    let inspected = worker::inspect(url).await;
    slot.release().await;
    let plan = match inspected {
        Ok(plan) => plan,
        // Nothing was learned, so the application goes back to where it was. Leaving it reading "preparing" would tell the owner work is in progress when none is.
        Err(e) => {
            applications::set_status(
                pool,
                application_id,
                &was,
                serde_json::json!({ "prepare_failed": e.to_string() }),
            )
            .await?;
            return Err(AppError::Invalid(worker_message(&e)));
        }
    };
    let facts = applications::approved_facts(pool).await?;
    let answers = applications::approved_answers(pool).await?;
    let mapping = map_form(&plan, &facts, &answers);
    // The review queue is rebuilt from the form itself, which keeps answers the owner has already given; the route is then decided with those answers counted.
    applications::upsert_review_tasks(pool, application_id, &mapping.open).await?;
    let mapping = with_answers(pool, application_id, mapping).await?;

    applications::set_route(pool, application_id, mapping.route, &mapping.reason).await?;

    let status = match mapping.route {
        Route::Auto => "ready",
        _ => "needs_review",
    };
    applications::set_status(
        pool,
        application_id,
        status,
        serde_json::json!({ "open_questions": mapping.open.len() }),
    )
    .await?;

    // One notification per transition into needing attention, deduplicated on the application and the question count so re-preparing an unchanged form does not notify twice.
    if mapping.route != Route::Auto {
        applications::notify(
            pool,
            Some(application_id),
            "action_required",
            &format!("action_required:{application_id}:{}", mapping.open.len()),
            serde_json::json!({ "route": mapping.route.as_str(), "reason": mapping.reason }),
        )
        .await?;
    }

    Ok(PrepareResult {
        application_id,
        route: mapping.route.as_str().to_owned(),
        reason: mapping.reason.clone(),
        mapped: mapping.mapped.len(),
        open_questions: mapping.open.len(),
        plan,
        mapping,
    })
}

/// The values that will be written into the form, taken from a mapping that `with_answers` has already settled.
pub fn payload_for(mapping: &FormMapping) -> Vec<worker::FillValue> {
    let mut values: Vec<worker::FillValue> = mapping
        .mapped
        .iter()
        .filter(|m| m.value != "<approved document>")
        .map(|m| worker::FillValue {
            field: m.field.clone(),
            value: m.value.clone(),
        })
        .collect();
    values.sort_by(|a, b| a.field.cmp(&b.field));
    values
}

/// Fingerprint of exactly what will be submitted.
///
/// The browser worker computes this identically in `hashValues`. Both sides must agree, because the worker refuses to submit when the payload it holds does not hash to the value the authorization was issued against. The definition is deliberately simple so two implementations in different languages can be checked against each other: fields sorted by name, one `field=value` per line, joined by newlines, SHA-256, lowercase hex.
pub fn payload_hash(values: &[worker::FillValue]) -> String {
    let mut sorted: Vec<&worker::FillValue> = values.iter().collect();
    sorted.sort_by(|a, b| a.field.cmp(&b.field));
    let canonical = sorted
        .iter()
        .map(|v| format!("{}={}", v.field, v.value))
        .collect::<Vec<_>>()
        .join("\n");
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn payload_hash_with_document(
    values: &[worker::FillValue],
    document_hash: Option<&str>,
) -> String {
    match document_hash {
        None => payload_hash(values),
        Some(hash) => format!(
            "{:x}",
            Sha256::digest(format!("{}\ncv={hash}", payload_hash(values)).as_bytes())
        ),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SubmitReport {
    pub application_id: Uuid,
    pub attempt_id: Uuid,
    pub permitted: bool,
    pub authorization: String,
    pub state: String,
    pub receipt: Option<String>,
    pub diagnostic: Option<String>,
    /// False when an existing attempt was reused, which is what a repeated request produces.
    pub newly_attempted: bool,
}

/// Submit an application, if and only if an authorization permits it right now.
///
/// The sequence matters and is the reason this is one function: the verdict is taken from current state, the attempt is claimed under a unique idempotency key in the same breath as spending the authorization, and only then does the browser open. A second request for the same payload finds the existing attempt and does nothing.
/// (paused, daily cap on automatic submissions), with defaults before the owner saves any setting.
pub async fn brakes(pool: &Db) -> Result<(bool, i32), AppError> {
    Ok(sqlx::query_as(
        "select coalesce(bool_or(submissions_paused), false), coalesce(min(max_automatic_per_day), 10) from product_settings",
    )
    .fetch_one(pool)
    .await?)
}

/// Owner-wide brakes above every employer policy: a pause stops all submissions, and a daily cap
/// limits automatic (policy) ones however many employers are authorized.
pub async fn submission_brake(pool: &Db, automatic: bool) -> Result<Option<String>, AppError> {
    let (paused, cap) = brakes(pool).await?;
    if paused {
        return Ok(Some("All submissions are paused by the owner.".into()));
    }
    if automatic {
        let today: i64 = sqlx::query_scalar("select count(*) from application_attempts where authorization_kind='policy' and started_at > now() - interval '1 day'")
            .fetch_one(pool)
            .await?;
        if today >= i64::from(cap) {
            return Ok(Some(format!(
                "The daily limit of {cap} automatic applications is reached."
            )));
        }
    }
    Ok(None)
}

async fn refuse_if_braked(
    pool: &Db,
    application_id: Uuid,
    automatic: bool,
) -> Result<(), AppError> {
    let Some(reason) = submission_brake(pool, automatic).await? else {
        return Ok(());
    };
    applications::record_event(
        pool,
        application_id,
        "submission_refused",
        serde_json::json!({ "reason": reason }),
    )
    .await?;
    Err(AppError::Invalid(reason))
}

pub async fn submit(
    pool: &Db,
    application_id: Uuid,
    url: &str,
    source_key: &str,
    document_path: Option<String>,
) -> Result<SubmitReport, AppError> {
    refuse_if_braked(pool, application_id, false).await?;
    // Held across the inspection, the authorization and the submission itself, so a sweep cannot start underneath a form that is halfway filled.
    let slot = claim_browser_slot(pool).await?;
    let report =
        submit_holding_the_machine(pool, application_id, url, source_key, document_path).await;
    slot.release().await;
    report
}

async fn submit_holding_the_machine(
    pool: &Db,
    application_id: Uuid,
    url: &str,
    source_key: &str,
    document_path: Option<String>,
) -> Result<SubmitReport, AppError> {
    // Checked inside the browser slot, which serializes submissions, so the status cannot change
    // between this check and the attempt row below.
    if !applications::destination_is_safe(url) {
        return Err(AppError::Invalid(
            "That destination is not an HTTPS employer address.".into(),
        ));
    }
    let status = applications::get_application(pool, application_id)
        .await?
        .status;
    if !applications::submittable(&status) {
        return Err(AppError::Invalid(format!(
            "An application that is {status} cannot be submitted again; resolve it first."
        )));
    }
    let plan = worker::inspect(url)
        .await
        .map_err(|e| AppError::Invalid(worker_message(&e)))?;
    if plan.has_captcha
        || plan.requires_login
        || plan.fields.is_empty()
        || plan.submit_selector.is_none()
    {
        return Err(AppError::Invalid(
            "This form requires manual completion.".into(),
        ));
    }
    let facts = applications::approved_facts(pool).await?;
    let answers = applications::approved_answers(pool).await?;
    let mapping = map_form(&plan, &facts, &answers);
    // Whether the form asked for personal writing is a property of the form, not of what is still unanswered, so a policy that forbids free text keeps refusing after the owner has written it.
    applications::upsert_review_tasks(pool, application_id, &mapping.open).await?;
    let has_free_text = mapping
        .open
        .iter()
        .any(|q| q.reason.contains("personal writing"));
    let mapping = with_answers(pool, application_id, mapping).await?;

    let values = payload_for(&mapping);
    let document_hash = document_path
        .as_ref()
        .map(|p| std::fs::read(p).map(|bytes| format!("{:x}", Sha256::digest(bytes))))
        .transpose()
        .map_err(|_| AppError::Invalid("The stored CV is unavailable. Please update it.".into()))?;
    let hash = payload_hash_with_document(&values, document_hash.as_deref());

    let context = applications::AuthorizationContext {
        application_id,
        source_key,
        destination: url,
        payload_hash: &hash,
        route: mapping.route,
        open_questions: mapping.open.len(),
        has_sensitive_question: mapping.open.iter().any(|q| q.sensitive),
        has_free_text,
    };
    let (decision, policy_id, token_id) =
        applications::decide_authorization(pool, &context).await?;

    if !decision.is_permitted() {
        applications::record_event(
            pool,
            application_id,
            "submission_refused",
            serde_json::json!({ "reason": decision.reason() }),
        )
        .await?;
        return Err(AppError::Invalid(decision.reason().to_owned()));
    }

    let kind = match &decision {
        opportunity_domain::Authorization::Policy { .. } => "policy",
        _ => "token",
    };
    // Inside the browser slot, so two automatic submissions cannot both pass the daily cap.
    refuse_if_braked(pool, application_id, kind == "policy").await?;
    // A standing policy is never broader than eligibility, whichever path the request came in by.
    // A one-time token is the owner's explicit decision about this exact submission, and is not held.
    if kind == "policy" {
        if let Some(reason) = crate::product::policy_eligibility_hold(pool, application_id).await? {
            applications::record_event(
                pool,
                application_id,
                "submission_refused",
                serde_json::json!({ "reason": reason }),
            )
            .await?;
            return Err(AppError::Invalid(reason));
        }
    }
    let (attempt, newly) =
        applications::begin_attempt(pool, application_id, url, &hash, kind, policy_id, token_id)
            .await?;

    if !newly {
        // A repeat of a request that already ran. Report what happened the first time rather than doing it again.
        return Ok(SubmitReport {
            application_id,
            attempt_id: attempt.id,
            permitted: true,
            authorization: decision.reason().to_owned(),
            state: attempt.state,
            receipt: attempt.receipt,
            diagnostic: Some(
                "this submission was already attempted; the earlier result is reported".into(),
            ),
            newly_attempted: false,
        });
    }

    applications::set_status(
        pool,
        application_id,
        "submitting",
        serde_json::json!({ "attempt": attempt.id }),
    )
    .await?;

    let reference = attempt.id.to_string();
    let marker = worker::submit_marker(&reference);
    let request = worker::SubmitRequest {
        fill: worker::FillRequest {
            url: url.to_owned(),
            values,
            document_path,
            reference,
            standing_authorized: true,
            document_hash,
        },
        authorized_payload_hash: hash.clone(),
        progress_path: marker.display().to_string(),
    };

    let report = |state: &str, receipt: Option<String>, diagnostic: Option<String>| SubmitReport {
        application_id,
        attempt_id: attempt.id,
        permitted: true,
        authorization: decision.reason().to_owned(),
        state: state.into(),
        receipt,
        diagnostic,
        newly_attempted: true,
    };
    let finish = |attempt_state: &'static str,
                  receipt: Option<String>,
                  diagnostic: Option<String>,
                  evidence: serde_json::Value| {
        let attempt_id = attempt.id;
        async move {
            applications::finish_submission(
                pool,
                application_id,
                attempt_id,
                applications::Outcome {
                    attempt_state,
                    receipt: receipt.as_deref(),
                    diagnostic: diagnostic.as_deref(),
                    evidence,
                },
            )
            .await
        }
    };

    match worker::submit(&request).await {
        Ok(worker::SubmitOutcome::Submitted {
            receipt,
            screenshot,
        }) => {
            finish(
                "submitted",
                Some(receipt.clone()),
                None,
                serde_json::json!({ "screenshot": screenshot }),
            )
            .await?;
            Ok(report("submitted", Some(receipt), None))
        }
        // The decisive case. Nobody knows whether this landed, so it is recorded as unknown and the duplicate guard now blocks any further attempt until a human says what happened.
        Ok(worker::SubmitOutcome::Unknown {
            diagnostic,
            screenshot,
        }) => {
            finish(
                "unknown",
                None,
                Some(diagnostic.clone()),
                serde_json::json!({ "screenshot": screenshot }),
            )
            .await?;
            Ok(report("submission_unknown", None, Some(diagnostic)))
        }
        // The worker only reports failure before it clicks; a failure after the click marker contradicts
        // that, and the conservative reading of a contradiction is "nobody knows".
        Ok(worker::SubmitOutcome::Failed { diagnostic }) if marker.exists() => {
            let diagnostic = format!("the worker reported a failure after the submit click, so whether it was recorded is unknown: {diagnostic}");
            finish(
                "unknown",
                None,
                Some(diagnostic.clone()),
                serde_json::json!({}),
            )
            .await?;
            Ok(report("submission_unknown", None, Some(diagnostic)))
        }
        // The worker reported a failure itself, which means it was alive to establish that nothing was
        // sent: the one outcome that leaves the application submittable again.
        Ok(worker::SubmitOutcome::Failed { diagnostic }) => {
            finish(
                "failed",
                None,
                Some(diagnostic.clone()),
                serde_json::json!({}),
            )
            .await?;
            Ok(report("failed", None, Some(diagnostic)))
        }
        // The worker did not report anything: it crashed, or the kernel killed it. Whether that is safe to retry depends entirely on whether it had already clicked, which is what the marker file records.
        Err(error) => {
            if marker.exists() {
                let diagnostic = format!(
                    "the browser died during or after the submission, so whether it was recorded is unknown: {}",
                    worker_message(&error)
                );
                finish(
                    "unknown",
                    None,
                    Some(diagnostic.clone()),
                    serde_json::json!({}),
                )
                .await?;
                return Ok(report("submission_unknown", None, Some(diagnostic)));
            }
            let diagnostic = format!(
                "the run ended before anything was submitted, so nothing was sent: {}",
                worker_message(&error)
            );
            finish(
                "failed",
                None,
                Some(diagnostic.clone()),
                serde_json::json!({}),
            )
            .await?;
            match error {
                // A deployment without a browser is not a failed attempt to use one; it is an answer about this deployment, and it reads better as one.
                worker::WorkerError::Unavailable(message) => Err(AppError::Invalid(message)),
                _ => Ok(report("failed", None, Some(diagnostic))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::FillValue;

    fn value(field: &str, value: &str) -> FillValue {
        FillValue {
            field: field.into(),
            value: value.into(),
        }
    }

    #[test]
    fn the_payload_hash_matches_the_definition_the_browser_worker_implements() {
        // Checked against the worker's own implementation of the same definition. If either side changes, this value changes and both must be updated together, which is the point of pinning it here.
        let values = vec![
            value("email", "a@example.test"),
            value("full name", "A. Example"),
        ];
        assert_eq!(
            payload_hash(&values),
            "18daee9cf2c2dbfcb124dad1d58e6658181b820aa1d431e84fb8322cb8215bdd"
        );
    }

    #[test]
    fn the_hash_is_order_independent_but_changes_with_any_answer() {
        let a = payload_hash(&[value("b", "2"), value("a", "1")]);
        let b = payload_hash(&[value("a", "1"), value("b", "2")]);
        let c = payload_hash(&[value("a", "1"), value("b", "changed")]);
        assert_eq!(a, b, "field order must not change the fingerprint");
        assert_ne!(a, c, "a changed answer must change the fingerprint");
    }
}
