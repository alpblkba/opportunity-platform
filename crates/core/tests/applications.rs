//! Session 4 integration tests: facts, routing, authorization, idempotency and notifications, against real PostgreSQL.
//!
//! The browser is exercised separately by the worker's own test suite against the mock ATS. These tests cover the decisions, which is where applying twice or submitting without permission would actually happen.

use chrono::Utc;
use opportunity_core::applications::{self, AuthorizationContext, NewFact, NewPolicy};
use opportunity_core::profiles;
use opportunity_core::workflow;
use opportunity_core::Db;
use opportunity_domain::{map_form, FieldKind, FormField, FormPlan, Profile, Route};
use sqlx::{Connection, Executor, PgConnection};
use uuid::Uuid;

struct TestDb {
    pool: Db,
    admin_url: String,
    name: String,
}

impl TestDb {
    async fn create() -> Option<Self> {
        let base = std::env::var("DATABASE_URL").ok()?;
        let (prefix, _) = base.rsplit_once('/')?;
        let admin_url = format!("{prefix}/postgres");
        let name = format!("opportunity_app_{}", Uuid::new_v4().simple());
        let mut admin = PgConnection::connect(&admin_url)
            .await
            .expect("connect to maintenance database");
        admin
            .execute(format!(r#"create database "{name}""#).as_str())
            .await
            .expect("create test database");
        let pool = opportunity_core::connect(&format!("{prefix}/{name}"))
            .await
            .expect("connect");
        opportunity_core::migrate(&pool).await.expect("migrate");
        Some(Self {
            pool,
            admin_url,
            name,
        })
    }

    async fn cleanup(self) {
        let Self {
            pool,
            admin_url,
            name,
        } = self;
        pool.close().await;
        if let Ok(mut admin) = PgConnection::connect(&admin_url).await {
            let _ = admin
                .execute(format!(r#"drop database if exists "{name}" with (force)"#).as_str())
                .await;
        }
    }
}

macro_rules! require_db {
    () => {
        match TestDb::create().await {
            Some(db) => db,
            None => {
                eprintln!("SKIPPED: DATABASE_URL is not set, so this PostgreSQL integration test did not run");
                return;
            }
        }
    };
}

/// A minimal opportunity to hang an application on.
async fn seed_opportunity(pool: &Db, title: &str) -> Uuid {
    let org: Uuid = sqlx::query_scalar(
        "insert into organizations (normalized_name, display_name) values ($1, $2)
         on conflict (normalized_name) do update set updated_at = now() returning id",
    )
    .bind("example silicon")
    .bind("Example Silicon")
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "insert into opportunities (organization_id, dedupe_key, canonical_title, kind, advertised_state, search_text)
         values ($1, $2, $3, 'employment', 'advertised', $3) returning id",
    )
    .bind(org)
    .bind(Uuid::new_v4().to_string())
    .bind(title)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn approved_identity_facts(pool: &Db) {
    for (key, value) in [
        ("full name", "A. Example"),
        ("email", "a@example.test"),
        ("phone", "+49 000 000"),
    ] {
        let fact = applications::upsert_fact(
            pool,
            NewFact {
                key: key.into(),
                value: value.into(),
                category: "identity".into(),
                sensitive: false,
                provenance: None,
            },
        )
        .await
        .unwrap();
        applications::approve_fact(pool, fact.id, true)
            .await
            .unwrap();
    }
}

fn text_field(name: &str) -> FormField {
    FormField {
        name: name.into(),
        label: name.into(),
        kind: FieldKind::Text,
        required: true,
        options: Vec::new(),
        max_length: Some(120),
    }
}

fn routine_plan() -> FormPlan {
    FormPlan {
        url: "http://127.0.0.1:8099/apply/single".into(),
        fields: vec![
            text_field("full name"),
            text_field("email"),
            text_field("phone"),
        ],
        steps: 1,
        has_captcha: false,
        requires_login: false,
        submit_selector: Some("button[type=submit]".into()),
    }
}

async fn enabled_policy(pool: &Db, source_key: &str) -> Uuid {
    let policy = applications::create_policy(
        pool,
        NewPolicy {
            name: "routine applications".into(),
            source_keys: vec![source_key.into()],
            max_per_day: Some(5),
            max_total: Some(20),
            expires_in_days: Some(30),
        },
    )
    .await
    .unwrap();
    // A policy is created disabled; enabling it is the owner's separate deliberate act.
    assert!(!policy.enabled, "a new policy must start disabled");
    applications::set_policy_state(pool, policy.id, true, false)
        .await
        .unwrap();
    policy.id
}

async fn context<'a>(
    application_id: Uuid,
    source_key: &'a str,
    payload_hash: &'a str,
    route: Route,
) -> AuthorizationContext<'a> {
    AuthorizationContext {
        application_id,
        source_key,
        destination: "http://127.0.0.1:8099/apply/single",
        payload_hash,
        route,
        open_questions: 0,
        has_sensitive_question: false,
        has_free_text: false,
    }
}

#[tokio::test]
async fn a_fact_must_be_approved_before_it_can_fill_a_form() {
    let db = require_db!();
    let fact = applications::upsert_fact(
        &db.pool,
        NewFact {
            key: "full name".into(),
            value: "A. Example".into(),
            category: "identity".into(),
            sensitive: false,
            provenance: None,
        },
    )
    .await
    .unwrap();
    assert!(!fact.approved, "a new fact starts unapproved");
    assert!(applications::approved_facts(&db.pool)
        .await
        .unwrap()
        .is_empty());

    applications::approve_fact(&db.pool, fact.id, true)
        .await
        .unwrap();
    assert_eq!(
        applications::approved_facts(&db.pool).await.unwrap().len(),
        1
    );

    // A new value supersedes rather than overwrites, and starts unapproved because the owner has not seen it.
    let updated = applications::upsert_fact(
        &db.pool,
        NewFact {
            key: "full name".into(),
            value: "A. Example-Changed".into(),
            category: "identity".into(),
            sensitive: false,
            provenance: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(updated.version, 2);
    assert!(!updated.approved);
    assert!(
        applications::approved_facts(&db.pool)
            .await
            .unwrap()
            .is_empty(),
        "an unapproved new version must not be usable, and the superseded one is no longer current"
    );
    assert_eq!(
        applications::list_facts(&db.pool, true)
            .await
            .unwrap()
            .len(),
        1,
        "exactly one version of a key is current at a time"
    );
    assert_eq!(
        applications::list_facts(&db.pool, false)
            .await
            .unwrap()
            .len(),
        2,
        "history is kept"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn a_routine_application_is_authorized_by_a_standing_policy_and_submits_exactly_once() {
    let db = require_db!();
    approved_identity_facts(&db.pool).await;
    let opportunity = seed_opportunity(&db.pool, "Firmware Engineer").await;
    let application = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();
    enabled_policy(&db.pool, "example-greenhouse").await;

    let mapping = map_form(
        &routine_plan(),
        &applications::approved_facts(&db.pool).await.unwrap(),
        &[],
    );
    assert_eq!(mapping.route, Route::Auto);
    let hash = "payload-1";

    let ctx = context(application.id, "example-greenhouse", hash, Route::Auto).await;
    let (decision, policy_id, token_id) = applications::decide_authorization(&db.pool, &ctx)
        .await
        .unwrap();
    assert!(decision.is_permitted(), "{}", decision.reason());
    assert!(policy_id.is_some());

    let (attempt, newly) = applications::begin_attempt(
        &db.pool,
        application.id,
        ctx.destination,
        hash,
        "policy",
        policy_id,
        token_id,
    )
    .await
    .unwrap();
    assert!(newly);
    applications::finish_attempt(
        &db.pool,
        attempt.id,
        "submitted",
        Some("receipt-1"),
        None,
        serde_json::json!({}),
    )
    .await
    .unwrap();

    // A second request for the same payload reuses the attempt rather than creating another.
    let (again, newly_again) = applications::begin_attempt(
        &db.pool,
        application.id,
        ctx.destination,
        hash,
        "policy",
        policy_id,
        token_id,
    )
    .await
    .unwrap();
    assert!(
        !newly_again,
        "a repeated submission must not create a second attempt"
    );
    assert_eq!(again.id, attempt.id);
    assert_eq!(
        applications::list_attempts(&db.pool, application.id)
            .await
            .unwrap()
            .len(),
        1
    );
    db.cleanup().await;
}

#[tokio::test]
async fn a_standard_thesis_application_follows_the_same_policy_as_an_employment_one() {
    let db = require_db!();
    approved_identity_facts(&db.pool).await;
    enabled_policy(&db.pool, "institut-academic").await;

    // Two opportunities of different kinds behind the same routine form shape.
    let thesis = seed_opportunity(&db.pool, "Masterarbeit: Speichersysteme").await;
    sqlx::query("update opportunities set kind = 'master_thesis' where id = $1")
        .bind(thesis)
        .execute(&db.pool)
        .await
        .unwrap();
    let thesis_app = applications::create_application(&db.pool, thesis, None)
        .await
        .unwrap();

    let ctx = context(
        thesis_app.id,
        "institut-academic",
        "payload-thesis",
        Route::Auto,
    )
    .await;
    let (decision, policy_id, _) = applications::decide_authorization(&db.pool, &ctx)
        .await
        .unwrap();
    assert!(
        decision.is_permitted(),
        "a thesis behind a routine form must be authorized exactly as any other: {}",
        decision.reason()
    );
    assert!(policy_id.is_some());
    db.cleanup().await;
}

#[tokio::test]
async fn an_unknown_answer_routes_to_review_and_is_never_guessed() {
    let db = require_db!();
    approved_identity_facts(&db.pool).await;
    let opportunity = seed_opportunity(&db.pool, "Firmware Engineer").await;
    let application = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();

    let mut plan = routine_plan();
    plan.fields.push(text_field("years of relevant experience"));
    let mapping = map_form(
        &plan,
        &applications::approved_facts(&db.pool).await.unwrap(),
        &[],
    );
    assert_eq!(mapping.route, Route::Review);

    applications::upsert_review_tasks(&db.pool, application.id, &mapping.open)
        .await
        .unwrap();
    let tasks = applications::list_review_tasks(&db.pool, Some(application.id))
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].field_name, "years of relevant experience");
    assert!(
        tasks[0].draft_answer.is_none(),
        "an unknown fact must never be drafted"
    );

    // A policy cannot authorize it while the question is open.
    enabled_policy(&db.pool, "example-greenhouse").await;
    let ctx = AuthorizationContext {
        open_questions: 1,
        ..context(
            application.id,
            "example-greenhouse",
            "payload-open",
            Route::Review,
        )
        .await
    };
    let (decision, _, _) = applications::decide_authorization(&db.pool, &ctx)
        .await
        .unwrap();
    assert!(!decision.is_permitted());

    // Answering it resolves the task and the answer persists for the fill payload.
    applications::answer_review_task(&db.pool, tasks[0].id, "4")
        .await
        .unwrap();
    assert!(
        applications::list_review_tasks(&db.pool, Some(application.id))
            .await
            .unwrap()
            .is_empty()
    );
    let answered = applications::answered_tasks(&db.pool, application.id)
        .await
        .unwrap();
    assert_eq!(
        answered,
        vec![("years of relevant experience".to_string(), "4".to_string())]
    );
    db.cleanup().await;
}

#[tokio::test]
async fn a_substantive_research_question_routes_to_curated_and_no_policy_will_submit_it() {
    let db = require_db!();
    approved_identity_facts(&db.pool).await;
    let opportunity = seed_opportunity(&db.pool, "Masterarbeit: Speichersysteme").await;
    let application = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();
    enabled_policy(&db.pool, "institut-academic").await;

    let mut plan = routine_plan();
    plan.fields.push(FormField {
        name: "statement".into(),
        label: "Describe your research interests".into(),
        kind: FieldKind::TextArea,
        required: true,
        options: Vec::new(),
        max_length: None,
    });
    let mapping = map_form(
        &plan,
        &applications::approved_facts(&db.pool).await.unwrap(),
        &[],
    );
    assert_eq!(mapping.route, Route::Curated);

    let ctx = AuthorizationContext {
        has_free_text: true,
        open_questions: 1,
        ..context(
            application.id,
            "institut-academic",
            "payload-curated",
            Route::Curated,
        )
        .await
    };
    let (decision, _, _) = applications::decide_authorization(&db.pool, &ctx)
        .await
        .unwrap();
    assert!(!decision.is_permitted());
    assert!(decision.reason().contains("needs you"));
    db.cleanup().await;
}

#[tokio::test]
async fn missing_or_revoked_authorization_prevents_submission() {
    let db = require_db!();
    approved_identity_facts(&db.pool).await;
    let opportunity = seed_opportunity(&db.pool, "Firmware Engineer").await;
    let application = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();
    let ctx = context(
        application.id,
        "example-greenhouse",
        "payload-1",
        Route::Auto,
    )
    .await;

    // Nothing configured.
    let (none, _, _) = applications::decide_authorization(&db.pool, &ctx)
        .await
        .unwrap();
    assert!(!none.is_permitted());
    assert!(none.reason().contains("no standing authorization exists"));

    // Created but not enabled.
    let policy = applications::create_policy(
        &db.pool,
        NewPolicy {
            name: "routine applications".into(),
            source_keys: vec!["example-greenhouse".into()],
            max_per_day: Some(5),
            max_total: None,
            expires_in_days: Some(30),
        },
    )
    .await
    .unwrap();
    let (disabled, _, _) = applications::decide_authorization(&db.pool, &ctx)
        .await
        .unwrap();
    assert!(
        !disabled.is_permitted(),
        "a policy must be enabled deliberately"
    );

    // Enabled, then revoked.
    applications::set_policy_state(&db.pool, policy.id, true, false)
        .await
        .unwrap();
    let (enabled, _, _) = applications::decide_authorization(&db.pool, &ctx)
        .await
        .unwrap();
    assert!(enabled.is_permitted());

    let revoked = applications::set_policy_state(&db.pool, policy.id, false, true)
        .await
        .unwrap();
    assert!(revoked.revoked_at.is_some());
    assert!(!revoked.enabled);
    let (after, _, _) = applications::decide_authorization(&db.pool, &ctx)
        .await
        .unwrap();
    assert!(!after.is_permitted(), "a revoked policy authorizes nothing");
    db.cleanup().await;
}

#[tokio::test]
async fn product_policy_is_bound_to_the_discovered_destination_and_profile() {
    let db = require_db!();
    profiles::seed_default_profiles(&db.pool).await.unwrap();
    let profile = profiles::get_by_key(&db.pool, "memory")
        .await
        .unwrap()
        .unwrap();
    let opportunity = seed_opportunity(&db.pool, "Memory engineer").await;
    let app = applications::create_application(&db.pool, opportunity, Some(profile.id))
        .await
        .unwrap();
    let policy = enabled_policy(&db.pool, "example-greenhouse").await;
    sqlx::query("update authorization_policies set profile_id=$2 where id=$1")
        .bind(policy)
        .bind(profile.id)
        .execute(&db.pool)
        .await
        .unwrap();
    let mut ctx = context(app.id, "example-greenhouse", "payload", Route::Auto).await;
    assert!(!applications::decide_authorization(&db.pool, &ctx)
        .await
        .unwrap()
        .0
        .is_permitted());
    let source: Uuid = sqlx::query_scalar("insert into sources(key,display_name,kind,endpoint) values('example-greenhouse','Example','greenhouse','{\"kind\":\"greenhouse\",\"board_token\":\"example\"}') returning id")
        .fetch_one(&db.pool).await.unwrap();
    sqlx::query("insert into postings(source_id,native_id,opportunity_id,url,title,content_hash) values($1,'example',$2,$3,'Memory engineer','example')")
        .bind(source).bind(opportunity).bind(ctx.destination).execute(&db.pool).await.unwrap();
    assert!(applications::decide_authorization(&db.pool, &ctx)
        .await
        .unwrap()
        .0
        .is_permitted());
    ctx.destination = "https://job-boards.greenhouse.io/another-employer/jobs/123";
    assert!(!applications::decide_authorization(&db.pool, &ctx)
        .await
        .unwrap()
        .0
        .is_permitted());
    ctx.destination = "http://127.0.0.1:8099/apply/single";
    sqlx::query("update applications set profile_id=null where id=$1")
        .bind(app.id)
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(!applications::decide_authorization(&db.pool, &ctx)
        .await
        .unwrap()
        .0
        .is_permitted());
    db.cleanup().await;
}

#[tokio::test]
async fn an_individual_token_is_one_use_and_bound_to_its_payload() {
    let db = require_db!();
    approved_identity_facts(&db.pool).await;
    let opportunity = seed_opportunity(&db.pool, "Firmware Engineer").await;
    let application = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();

    let token = applications::issue_token(
        &db.pool,
        application.id,
        "http://127.0.0.1:8099/apply/single",
        "example-greenhouse",
        "payload-1",
        60,
    )
    .await
    .unwrap();

    // The same token does not cover different answers.
    let wrong = context(
        application.id,
        "example-greenhouse",
        "payload-changed",
        Route::Auto,
    )
    .await;
    let (refused, _, _) = applications::decide_authorization(&db.pool, &wrong)
        .await
        .unwrap();
    assert!(!refused.is_permitted());
    assert!(refused.reason().contains("answers changed"));

    let right = context(
        application.id,
        "example-greenhouse",
        "payload-1",
        Route::Auto,
    )
    .await;
    let (permitted, _, token_id) = applications::decide_authorization(&db.pool, &right)
        .await
        .unwrap();
    assert!(permitted.is_permitted());
    assert_eq!(token_id, Some(token));

    // Spending it marks it used, and it cannot be spent again.
    let (attempt, _) = applications::begin_attempt(
        &db.pool,
        application.id,
        right.destination,
        "payload-1",
        "token",
        None,
        token_id,
    )
    .await
    .unwrap();
    applications::finish_attempt(
        &db.pool,
        attempt.id,
        "failed",
        None,
        Some("mock failure"),
        serde_json::json!({}),
    )
    .await
    .unwrap();

    let used: Option<chrono::DateTime<Utc>> =
        sqlx::query_scalar("select used_at from authorization_tokens where id = $1")
            .bind(token)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(used.is_some(), "spending a token must mark it used");
    let (after, _, _) = applications::decide_authorization(&db.pool, &right)
        .await
        .unwrap();
    assert!(
        !after.is_permitted(),
        "a spent token authorizes nothing further"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn an_uncertain_outcome_blocks_any_further_attempt_until_a_human_resolves_it() {
    let db = require_db!();
    approved_identity_facts(&db.pool).await;
    let opportunity = seed_opportunity(&db.pool, "Firmware Engineer").await;
    let application = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();
    enabled_policy(&db.pool, "example-greenhouse").await;

    let ctx = context(
        application.id,
        "example-greenhouse",
        "payload-1",
        Route::Auto,
    )
    .await;
    let (_, policy_id, _) = applications::decide_authorization(&db.pool, &ctx)
        .await
        .unwrap();
    let (attempt, _) = applications::begin_attempt(
        &db.pool,
        application.id,
        ctx.destination,
        "payload-1",
        "policy",
        policy_id,
        None,
    )
    .await
    .unwrap();
    applications::finish_attempt(
        &db.pool,
        attempt.id,
        "unknown",
        None,
        Some("the site did not answer"),
        serde_json::json!({}),
    )
    .await
    .unwrap();

    // A different payload would otherwise get a fresh idempotency key, so the duplicate guard has to be independent of it.
    let different = context(
        application.id,
        "example-greenhouse",
        "payload-2",
        Route::Auto,
    )
    .await;
    let (decision, _, _) = applications::decide_authorization(&db.pool, &different)
        .await
        .unwrap();
    assert!(
        !decision.is_permitted(),
        "an unresolved uncertain outcome must block retries"
    );
    assert!(decision.reason().contains("could apply twice"));
    db.cleanup().await;
}

#[tokio::test]
async fn a_failed_attempt_leaves_the_application_submittable_because_nothing_was_sent() {
    let db = require_db!();
    approved_identity_facts(&db.pool).await;
    let opportunity = seed_opportunity(&db.pool, "Firmware Engineer").await;
    let application = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();
    enabled_policy(&db.pool, "example-greenhouse").await;

    let ctx = context(
        application.id,
        "example-greenhouse",
        "payload-1",
        Route::Auto,
    )
    .await;
    let (_, policy_id, _) = applications::decide_authorization(&db.pool, &ctx)
        .await
        .unwrap();
    let (attempt, _) = applications::begin_attempt(
        &db.pool,
        application.id,
        ctx.destination,
        "payload-1",
        "policy",
        policy_id,
        None,
    )
    .await
    .unwrap();
    applications::finish_attempt(
        &db.pool,
        attempt.id,
        "failed",
        None,
        Some("no submit control"),
        serde_json::json!({}),
    )
    .await
    .unwrap();

    let retry = context(
        application.id,
        "example-greenhouse",
        "payload-2",
        Route::Auto,
    )
    .await;
    let (decision, _, _) = applications::decide_authorization(&db.pool, &retry)
        .await
        .unwrap();
    assert!(
        decision.is_permitted(),
        "a failure sent nothing, so trying again is safe: {}",
        decision.reason()
    );
    db.cleanup().await;
}

#[tokio::test]
async fn a_policy_stops_at_its_daily_limit_counted_from_real_attempts() {
    let db = require_db!();
    approved_identity_facts(&db.pool).await;
    let policy = applications::create_policy(
        &db.pool,
        NewPolicy {
            name: "two a day".into(),
            source_keys: vec!["example-greenhouse".into()],
            max_per_day: Some(2),
            max_total: None,
            expires_in_days: Some(30),
        },
    )
    .await
    .unwrap();
    applications::set_policy_state(&db.pool, policy.id, true, false)
        .await
        .unwrap();

    for index in 0..2 {
        let opportunity = seed_opportunity(&db.pool, &format!("Role {index}")).await;
        let application = applications::create_application(&db.pool, opportunity, None)
            .await
            .unwrap();
        let payload = format!("payload-{index}");
        let ctx = context(application.id, "example-greenhouse", &payload, Route::Auto).await;
        let (decision, policy_id, _) = applications::decide_authorization(&db.pool, &ctx)
            .await
            .unwrap();
        assert!(
            decision.is_permitted(),
            "submission {index} should be allowed"
        );
        let (attempt, _) = applications::begin_attempt(
            &db.pool,
            application.id,
            ctx.destination,
            &payload,
            "policy",
            policy_id,
            None,
        )
        .await
        .unwrap();
        applications::finish_attempt(
            &db.pool,
            attempt.id,
            "submitted",
            Some("r"),
            None,
            serde_json::json!({}),
        )
        .await
        .unwrap();
    }

    let third = seed_opportunity(&db.pool, "Role 3").await;
    let application = applications::create_application(&db.pool, third, None)
        .await
        .unwrap();
    let ctx = context(
        application.id,
        "example-greenhouse",
        "payload-3",
        Route::Auto,
    )
    .await;
    let (decision, _, _) = applications::decide_authorization(&db.pool, &ctx)
        .await
        .unwrap();
    assert!(!decision.is_permitted());
    assert!(decision.reason().contains("daily limit"));
    db.cleanup().await;
}

#[tokio::test]
async fn a_notification_is_emitted_exactly_once_per_transition() {
    let db = require_db!();
    let opportunity = seed_opportunity(&db.pool, "Firmware Engineer").await;
    let application = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();

    let first = applications::notify(
        &db.pool,
        Some(application.id),
        "action_required",
        &format!("action_required:{}:1", application.id),
        serde_json::json!({ "reason": "one question" }),
    )
    .await
    .unwrap();
    assert!(first, "the first emission creates a notification");

    let repeat = applications::notify(
        &db.pool,
        Some(application.id),
        "action_required",
        &format!("action_required:{}:1", application.id),
        serde_json::json!({ "reason": "one question" }),
    )
    .await
    .unwrap();
    assert!(!repeat, "the same transition must not notify twice");
    assert_eq!(
        applications::list_notifications(&db.pool, false)
            .await
            .unwrap()
            .len(),
        1
    );

    // A genuinely different transition does notify.
    applications::notify(
        &db.pool,
        Some(application.id),
        "submitted",
        &format!("submitted:{}", Uuid::new_v4()),
        serde_json::json!({}),
    )
    .await
    .unwrap();
    let all = applications::list_notifications(&db.pool, false)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(
        applications::list_notifications(&db.pool, true)
            .await
            .unwrap()
            .len(),
        2
    );

    applications::mark_delivered(&db.pool, all[0].id)
        .await
        .unwrap();
    assert_eq!(
        applications::list_notifications(&db.pool, true)
            .await
            .unwrap()
            .len(),
        1
    );
    db.cleanup().await;
}

#[tokio::test]
async fn review_tasks_and_application_state_survive_a_restart() {
    let db = require_db!();
    approved_identity_facts(&db.pool).await;
    let opportunity = seed_opportunity(&db.pool, "Firmware Engineer").await;
    let application = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();

    let mut plan = routine_plan();
    plan.fields.push(text_field("years of relevant experience"));
    let mapping = map_form(
        &plan,
        &applications::approved_facts(&db.pool).await.unwrap(),
        &[],
    );
    applications::upsert_review_tasks(&db.pool, application.id, &mapping.open)
        .await
        .unwrap();
    applications::set_status(
        &db.pool,
        application.id,
        "needs_review",
        serde_json::json!({}),
    )
    .await
    .unwrap();
    applications::set_route(
        &db.pool,
        application.id,
        Route::Review,
        "a question needs answering",
    )
    .await
    .unwrap();

    let task = applications::list_review_tasks(&db.pool, Some(application.id))
        .await
        .unwrap()[0]
        .id;
    applications::answer_review_task(&db.pool, task, "4")
        .await
        .unwrap();

    // Everything a restarted process would read back.
    let reloaded = applications::get_application(&db.pool, application.id)
        .await
        .unwrap();
    assert_eq!(reloaded.status, "needs_review");
    assert_eq!(reloaded.route.as_deref(), Some("review"));
    assert_eq!(
        reloaded.route_reason.as_deref(),
        Some("a question needs answering")
    );
    assert_eq!(
        applications::answered_tasks(&db.pool, application.id)
            .await
            .unwrap(),
        vec![("years of relevant experience".to_string(), "4".to_string())]
    );
    let events = applications::list_events(&db.pool, application.id)
        .await
        .unwrap();
    assert!(events.iter().any(|e| e.kind == "review_answered"));
    assert!(events.iter().any(|e| e.kind == "routed"));
    db.cleanup().await;
}

#[tokio::test]
async fn one_application_per_opportunity_is_a_database_guarantee() {
    let db = require_db!();
    let opportunity = seed_opportunity(&db.pool, "Firmware Engineer").await;
    let first = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();
    let second = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();
    assert_eq!(
        first.id, second.id,
        "a second create must return the same application"
    );
    let count: i64 = sqlx::query_scalar("select count(*) from applications")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    db.cleanup().await;
}

#[tokio::test]
async fn profiles_and_discovery_from_earlier_sessions_still_work_alongside_applications() {
    let db = require_db!();
    // Session 3 behaviour must survive the session 4 migration.
    let created = profiles::seed_default_profiles(&db.pool).await.unwrap();
    assert_eq!(created.len(), 5);
    let mut profile = Profile::blank("custom", "Custom");
    profile.terms = vec!["DRAM".into()];
    let stored = profiles::create(&db.pool, profile).await.unwrap();
    assert_eq!(
        profiles::get(&db.pool, stored.id)
            .await
            .unwrap()
            .profile
            .terms,
        vec!["DRAM"]
    );
    assert_eq!(profiles::list(&db.pool).await.unwrap().len(), 6);
    db.cleanup().await;
}

#[tokio::test]
async fn answering_a_review_question_never_marks_an_application_submitted() {
    let db = require_db!();
    approved_identity_facts(&db.pool).await;
    let opportunity = seed_opportunity(&db.pool, "Firmware Engineer").await;
    let application = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();

    let mut plan = routine_plan();
    plan.fields.push(text_field("years of relevant experience"));
    let mapping = map_form(
        &plan,
        &applications::approved_facts(&db.pool).await.unwrap(),
        &[],
    );
    applications::upsert_review_tasks(&db.pool, application.id, &mapping.open)
        .await
        .unwrap();
    applications::set_status(
        &db.pool,
        application.id,
        "needs_review",
        serde_json::json!({}),
    )
    .await
    .unwrap();

    let task = applications::list_review_tasks(&db.pool, Some(application.id))
        .await
        .unwrap()[0]
        .id;
    applications::answer_review_task(&db.pool, task, "4")
        .await
        .unwrap();

    // Resolving a question is not applying. The status must be untouched and no attempt may exist.
    let after = applications::get_application(&db.pool, application.id)
        .await
        .unwrap();
    assert_eq!(
        after.status, "needs_review",
        "answering a question must not change submission status"
    );
    assert!(applications::list_attempts(&db.pool, application.id)
        .await
        .unwrap()
        .is_empty());
    let events = applications::list_events(&db.pool, application.id)
        .await
        .unwrap();
    assert!(
        !events.iter().any(|e| e.kind.contains("submitted")),
        "no submission event may be written by answering a question"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn an_application_is_only_submitted_when_an_attempt_carries_real_evidence() {
    let db = require_db!();
    approved_identity_facts(&db.pool).await;
    let opportunity = seed_opportunity(&db.pool, "Firmware Engineer").await;
    let application = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();
    enabled_policy(&db.pool, "example-greenhouse").await;

    let ctx = context(
        application.id,
        "example-greenhouse",
        "payload-1",
        Route::Auto,
    )
    .await;
    let (_, policy_id, _) = applications::decide_authorization(&db.pool, &ctx)
        .await
        .unwrap();
    let (attempt, _) = applications::begin_attempt(
        &db.pool,
        application.id,
        ctx.destination,
        "payload-1",
        "policy",
        policy_id,
        None,
    )
    .await
    .unwrap();

    // Starting an attempt is not evidence of arrival, so the application is not submitted yet.
    assert_eq!(
        applications::get_application(&db.pool, application.id)
            .await
            .unwrap()
            .status,
        "saved"
    );

    applications::finish_attempt(
        &db.pool,
        attempt.id,
        "submitted",
        Some("receipt-1"),
        None,
        serde_json::json!({}),
    )
    .await
    .unwrap();
    applications::set_status(
        &db.pool,
        application.id,
        "submitted",
        serde_json::json!({ "receipt": "receipt-1" }),
    )
    .await
    .unwrap();

    let attempts = applications::list_attempts(&db.pool, application.id)
        .await
        .unwrap();
    assert_eq!(attempts[0].state, "submitted");
    assert_eq!(
        attempts[0].receipt.as_deref(),
        Some("receipt-1"),
        "submitted status must be backed by a receipt"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn answering_every_open_question_makes_the_application_automatable_and_shows_the_answers() {
    let db = require_db!();
    approved_identity_facts(&db.pool).await;
    let opportunity = seed_opportunity(&db.pool, "Embedded Systems Intern").await;
    let application = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();

    let mut plan = routine_plan();
    plan.fields.push(text_field("notice period"));
    let mapping = map_form(
        &plan,
        &applications::approved_facts(&db.pool).await.unwrap(),
        &[],
    );
    assert_eq!(mapping.route, Route::Review);
    applications::upsert_review_tasks(&db.pool, application.id, &mapping.open)
        .await
        .unwrap();

    // Before the owner answers, settling changes nothing: the question is still theirs.
    let pending = workflow::with_answers(&db.pool, application.id, mapping.clone())
        .await
        .unwrap();
    assert_eq!(pending.route, Route::Review);

    let tasks = applications::list_review_tasks(&db.pool, Some(application.id))
        .await
        .unwrap();
    applications::answer_review_task(&db.pool, tasks[0].id, "3 months")
        .await
        .unwrap();

    // Afterwards it is automatable as far as the form goes, and the answer appears where the owner can see what will be sent.
    let settled = workflow::with_answers(&db.pool, application.id, mapping)
        .await
        .unwrap();
    assert_eq!(settled.route, Route::Auto);
    assert!(settled.open.is_empty());
    let filled = settled
        .mapped
        .iter()
        .find(|m| m.field == "notice period")
        .expect("the answer is filled like any other value");
    assert_eq!(filled.value, "3 months");
    assert!(filled.source.contains("review queue"));
    // What the owner is shown is what gets hashed and submitted.
    assert!(workflow::payload_for(&settled)
        .iter()
        .any(|v| v.field == "notice period" && v.value == "3 months"));
    db.cleanup().await;
}

#[tokio::test]
async fn applying_to_the_same_opportunity_twice_does_not_record_a_second_birth() {
    let db = require_db!();
    let opportunity = seed_opportunity(&db.pool, "Firmware Engineer").await;
    let first = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();
    let second = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();
    assert_eq!(first.id, second.id, "one application per opportunity");

    let births = applications::list_events(&db.pool, first.id)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == "created")
        .count();
    assert_eq!(births, 1, "an application's history has one beginning");
    db.cleanup().await;
}

#[tokio::test]
async fn a_form_that_cannot_be_inspected_leaves_the_application_where_it_was() {
    let db = require_db!();
    let opportunity = seed_opportunity(&db.pool, "Firmware Engineer").await;
    let application = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();
    let before = application.status.clone();

    // Nothing is listening there, so the inspection fails however it is attempted.
    let outcome = workflow::prepare(&db.pool, application.id, "http://127.0.0.1:9/apply").await;
    assert!(outcome.is_err(), "an unreachable form is not a preparation");

    let after = applications::get_application(&db.pool, application.id)
        .await
        .unwrap();
    assert_eq!(
        after.status, before,
        "a failed inspection must not leave the application reading as in progress"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn the_same_answers_may_be_sent_again_after_a_failure_but_never_after_a_success_or_an_unknown(
) {
    let db = require_db!();
    approved_identity_facts(&db.pool).await;
    let opportunity = seed_opportunity(&db.pool, "Firmware Engineer").await;
    let application = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();

    let claim = |payload: &'static str| {
        let pool = db.pool.clone();
        let id = application.id;
        async move {
            applications::begin_attempt(
                &pool,
                id,
                "http://127.0.0.1:8099/apply/single",
                payload,
                "token",
                None,
                None,
            )
            .await
            .unwrap()
        }
    };

    // A failure means the form refused or the browser died before anything was sent, so the owner may try the same answers again rather than having to change one.
    let (first, newly) = claim("payload-1").await;
    assert!(newly);
    applications::finish_attempt(
        &db.pool,
        first.id,
        "failed",
        None,
        Some("the browser crashed"),
        serde_json::json!({}),
    )
    .await
    .unwrap();

    let (second, newly) = claim("payload-1").await;
    assert!(newly, "nothing was sent, so this is a new attempt");
    assert_ne!(
        second.id, first.id,
        "the failed attempt stays in the history"
    );

    // An outcome nobody can verify is final until a human resolves it.
    applications::finish_attempt(
        &db.pool,
        second.id,
        "unknown",
        None,
        Some("the site never answered"),
        serde_json::json!({}),
    )
    .await
    .unwrap();
    let (again, newly) = claim("payload-1").await;
    assert!(
        !newly,
        "an unverified outcome must never be retried blindly"
    );
    assert_eq!(again.id, second.id);

    // And a success is final for the obvious reason.
    let (other, newly) = claim("payload-2").await;
    assert!(newly);
    applications::finish_attempt(
        &db.pool,
        other.id,
        "submitted",
        Some("receipt-1"),
        None,
        serde_json::json!({}),
    )
    .await
    .unwrap();
    let (repeat, newly) = claim("payload-2").await;
    assert!(!newly, "applying twice is the thing this prevents");
    assert_eq!(repeat.receipt.as_deref(), Some("receipt-1"));
    db.cleanup().await;
}

/// Point the worker bridge at a script that dies instead of at the real browser, and give the
/// artefacts their own directory so one test cannot see another's marker.
struct DeadWorker {
    artefacts: std::path::PathBuf,
}

impl DeadWorker {
    fn install(mode: &str) -> Self {
        let artefacts =
            std::env::temp_dir().join(format!("opportunity-dead-worker-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&artefacts).expect("artefact directory");
        std::env::set_var("BROWSER_WORKER_ENTRY", "tests/fake-worker/cli.js");
        std::env::set_var("BROWSER_WORKER_ARTEFACTS", &artefacts);
        std::env::set_var("FAKE_WORKER_MODE", mode);
        Self { artefacts }
    }
}

impl Drop for DeadWorker {
    fn drop(&mut self) {
        std::env::remove_var("BROWSER_WORKER_ENTRY");
        std::env::remove_var("BROWSER_WORKER_ARTEFACTS");
        std::env::remove_var("FAKE_WORKER_MODE");
        let _ = std::fs::remove_dir_all(&self.artefacts);
    }
}

async fn application_ready_to_submit(pool: &Db, title: &str) -> Uuid {
    approved_identity_facts(pool).await;
    let opportunity = seed_opportunity(pool, title).await;
    applications::create_application(pool, opportunity, None)
        .await
        .unwrap()
        .id
}

/// Both halves in one test, deliberately: the fake worker is selected with process-wide environment
/// variables, and two tests setting them at once would be testing each other.
#[tokio::test]
async fn a_killed_worker_is_a_failure_only_when_it_is_proven_not_to_have_clicked() {
    let db = require_db!();
    let destination = "http://127.0.0.1:8099/apply/single";

    // Killed before the click. The marker the real worker writes immediately before clicking does
    // not exist, so nothing was sent and the same answers may go again.
    {
        let _worker = DeadWorker::install("die_before_clicking");
        let application = application_ready_to_submit(&db.pool, "Firmware Engineer").await;
        let payload = dead_worker_payload_hash(&db.pool, application).await;
        applications::issue_token(&db.pool, application, destination, "mock", &payload, 5)
            .await
            .unwrap();

        let report = workflow::submit(&db.pool, application, destination, "mock", None)
            .await
            .expect("a dead worker is an outcome, not an error");
        assert_eq!(
            report.state, "failed",
            "nothing was clicked, so nothing was sent: {:?}",
            report.diagnostic
        );
        assert!(report.receipt.is_none());
        assert!(
            report
                .diagnostic
                .as_deref()
                .unwrap()
                .contains("nothing was sent"),
            "{:?}",
            report.diagnostic
        );

        let (_, newly) = applications::begin_attempt(
            &db.pool,
            application,
            destination,
            &payload,
            "token",
            None,
            None,
        )
        .await
        .unwrap();
        assert!(newly, "a proven non-submission is retryable");
    }

    // Killed after the click. Nobody can say the form did not go, so this is uncertain, it reaches
    // the owner, and nothing retries it.
    {
        let _worker = DeadWorker::install("die_after_clicking");
        let application = application_ready_to_submit(&db.pool, "Platform Engineer").await;
        let payload = dead_worker_payload_hash(&db.pool, application).await;
        applications::issue_token(&db.pool, application, destination, "mock", &payload, 5)
            .await
            .unwrap();

        let report = workflow::submit(&db.pool, application, destination, "mock", None)
            .await
            .expect("a dead worker is an outcome, not an error");
        assert_eq!(
            report.state, "submission_unknown",
            "the click happened, so nobody can say it was not sent: {:?}",
            report.diagnostic
        );
        assert!(report.receipt.is_none());

        let stored = applications::get_application(&db.pool, application)
            .await
            .unwrap();
        assert_eq!(stored.status, "submission_unknown");
        let notifications = applications::list_notifications(&db.pool, true)
            .await
            .unwrap();
        assert!(
            notifications
                .iter()
                .any(|n| n.kind == "submission_uncertain"),
            "an uncertain outcome has to reach the owner"
        );

        let (_, newly) = applications::begin_attempt(
            &db.pool,
            application,
            destination,
            &payload,
            "token",
            None,
            None,
        )
        .await
        .unwrap();
        assert!(
            !newly,
            "an unverified outcome must never be retried blindly"
        );
    }
    db.cleanup().await;
}

/// The payload hash `submit` will compute for this application against the fake worker's plan.
async fn dead_worker_payload_hash(pool: &Db, application_id: Uuid) -> String {
    let plan = opportunity_core::worker::inspect("http://127.0.0.1:8099/apply/single")
        .await
        .expect("the fake worker answers inspect");
    let facts = applications::approved_facts(pool).await.unwrap();
    let answers = applications::approved_answers(pool).await.unwrap();
    let mapping = map_form(&plan, &facts, &answers);
    let mapping = workflow::with_answers(pool, application_id, mapping)
        .await
        .unwrap();
    workflow::payload_hash(&workflow::payload_for(&mapping))
}

#[tokio::test]
async fn manual_tracking_persists_notes_and_owner_history_without_a_submission_attempt() {
    let db = require_db!();
    let opportunity = seed_opportunity(&db.pool, "Synthetic tracked role").await;
    let app = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();
    let duplicate = applications::create_application(&db.pool, opportunity, None)
        .await
        .unwrap();
    assert_eq!(app.id, duplicate.id);
    let updated = applications::update_tracking(
        &db.pool,
        app.id,
        "submitted",
        "Applied on employer website myself",
    )
    .await
    .unwrap();
    assert_eq!(updated.status, "submitted");
    assert_eq!(
        updated.notes.as_deref(),
        Some("Applied on employer website myself")
    );
    let events = applications::list_events(&db.pool, app.id).await.unwrap();
    let event = events
        .iter()
        .find(|event| event.kind == "manual_tracking")
        .unwrap();
    assert_eq!(event.detail["reported_by"], "owner");
    assert_eq!(event.detail["submission_performed"], false);
    assert!(applications::list_attempts(&db.pool, app.id)
        .await
        .unwrap()
        .is_empty());
    assert!(
        applications::update_tracking(&db.pool, app.id, "submitting", "")
            .await
            .is_err()
    );
    assert_eq!(
        applications::get_application(&db.pool, app.id)
            .await
            .unwrap()
            .status,
        "submitted"
    );
    applications::set_status(&db.pool, app.id, "submitting", serde_json::json!({}))
        .await
        .unwrap();
    assert!(applications::update_tracking(&db.pool, app.id, "saved", "")
        .await
        .is_err());
    db.cleanup().await;
}
