//! The owner's workspace choices over real PostgreSQL with synthetic data: marks, dismissals and
//! blocked employers, filters, notification preferences, the new-match digest and deadline reminders.

use opportunity_core::{alerts, applications, preferences, product, profiles, Db};
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
        let name = format!("opportunity_test_{}", Uuid::new_v4().simple());
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
        self.pool.close().await;
        if let Ok(mut admin) = PgConnection::connect(&self.admin_url).await {
            let _ = admin
                .execute(
                    format!(r#"drop database if exists "{}" with (force)"#, self.name).as_str(),
                )
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

async fn setup(pool: &Db) {
    profiles::seed_default_profiles(pool).await.unwrap();
    let candidate = serde_json::json!({
        "name": null, "email": null, "phone": null, "degree": null, "experience_years": 2,
        "education": ["M.Sc. Computer Science (synthetic)"], "employment": ["Network software engineer (synthetic)"],
        "projects": ["Bootloader for a RISC-V board (synthetic)"], "skills": ["C", "C++", "Linux", "RTOS", "RISC-V"],
    });
    sqlx::query("insert into documents (owner_id, kind, label, file_path, content_hash, byte_size, media_type, candidate, approved)
                 select id, 'cv', 'synthetic.txt', 'private/documents/synthetic.txt', 'synthetic', 1, 'text/plain', $1, true from owners limit 1")
        .bind(candidate).execute(pool).await.unwrap();
}

async fn opportunity(
    pool: &Db,
    employer: &str,
    title: &str,
    kind: &str,
    country: &str,
    deadline_days: Option<f64>,
) -> Uuid {
    let org: Uuid = sqlx::query_scalar("insert into organizations (normalized_name, display_name) values (lower($1), $1) on conflict (normalized_name) do update set updated_at = now() returning id")
        .bind(employer).fetch_one(pool).await.unwrap();
    let source: Uuid = sqlx::query_scalar("insert into sources (key, display_name, kind, endpoint) values ('example-lever', 'Example', 'lever', '{\"kind\":\"lever\",\"site\":\"example\"}') on conflict (key) do update set display_name = excluded.display_name returning id")
        .fetch_one(pool).await.unwrap();
    let description = "Embedded C firmware on an RTOS for RISC-V boards; bootloader bring-up.";
    let id: Uuid = sqlx::query_scalar(
        "insert into opportunities (organization_id, dedupe_key, canonical_title, kind, advertised_state, search_text, country_code, deadline_at)
         values ($1, $2, $3, $4, 'advertised', $3 || ' ' || $5, $6, now() + make_interval(secs => $7)) returning id")
        .bind(org).bind(Uuid::new_v4().to_string()).bind(title).bind(kind).bind(description).bind(country)
        .bind(deadline_days.map(|d| d * 86400.0))
        .fetch_one(pool).await.unwrap();
    sqlx::query("insert into postings (source_id, native_id, opportunity_id, url, title, description_text, content_hash) values ($1, $2, $3, $4, $5, $6, 'x')")
        .bind(source).bind(id.to_string()).bind(id).bind(format!("https://jobs.lever.co/example/{id}")).bind(title).bind(description)
        .execute(pool).await.unwrap();
    id
}

async fn ranked_titles(pool: &Db, filters: &product::Filters) -> Vec<String> {
    let home = product::dashboard_with(pool, filters).await.unwrap();
    home["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["canonical_title"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn marks_dismissals_and_blocked_employers_persist_and_can_be_undone() {
    let db = require_db!();
    setup(&db.pool).await;
    let intern = opportunity(
        &db.pool,
        "Example Silicon",
        "Firmware Engineering Intern",
        "internship",
        "DE",
        None,
    )
    .await;
    let other = opportunity(
        &db.pool,
        "Other Chips",
        "Embedded Firmware Engineer",
        "employment",
        "NL",
        None,
    )
    .await;
    let all = product::Filters::default();
    assert_eq!(ranked_titles(&db.pool, &all).await.len(), 2);

    preferences::set_mark(&db.pool, intern, "saved", true)
        .await
        .unwrap();
    preferences::set_mark(&db.pool, intern, "saved", true)
        .await
        .unwrap();
    preferences::set_mark(&db.pool, other, "review", true)
        .await
        .unwrap();
    assert!(preferences::set_mark(&db.pool, intern, "favourite", true)
        .await
        .is_err());
    assert!(
        preferences::set_mark(&db.pool, Uuid::new_v4(), "saved", true)
            .await
            .is_err()
    );
    let saved_only = product::Filters {
        saved: true,
        ..Default::default()
    };
    assert_eq!(
        ranked_titles(&db.pool, &saved_only).await,
        vec!["Firmware Engineering Intern"]
    );
    let home = product::dashboard(&db.pool).await.unwrap();
    assert_eq!(home["marks"].as_array().unwrap().len(), 2);

    // Dismissing hides it from every profile; deleting the rule brings it back.
    let rule = preferences::dismiss(&db.pool, intern).await.unwrap();
    assert_eq!(
        ranked_titles(&db.pool, &all).await,
        vec!["Embedded Firmware Engineer"]
    );
    profiles::delete_rule(&db.pool, rule.id).await.unwrap();
    assert_eq!(ranked_titles(&db.pool, &all).await.len(), 2);

    // Blocking an employer keeps its postings and source, and only stops ranking them.
    let block = preferences::block_employer(&db.pool, "  Other Chips ")
        .await
        .unwrap();
    assert_eq!(
        ranked_titles(&db.pool, &all).await,
        vec!["Firmware Engineering Intern"]
    );
    let kept: i64 = sqlx::query_scalar("select count(*) from postings")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(kept, 2);
    let home = product::dashboard(&db.pool).await.unwrap();
    assert!(home["exclusions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["kind"] == "company" && r["value"] == "Other Chips"));
    profiles::delete_rule(&db.pool, block.id).await.unwrap();
    assert!(preferences::block_employer(&db.pool, "   ").await.is_err());

    preferences::set_mark(&db.pool, intern, "saved", false)
        .await
        .unwrap();
    assert!(ranked_titles(&db.pool, &saved_only).await.is_empty());
    db.cleanup().await;
}

#[tokio::test]
async fn filters_narrow_every_match_not_just_the_page_and_report_their_options() {
    let db = require_db!();
    setup(&db.pool).await;
    opportunity(
        &db.pool,
        "Example Silicon",
        "Firmware Engineering Intern",
        "internship",
        "DE",
        None,
    )
    .await;
    opportunity(
        &db.pool,
        "Other Chips",
        "Embedded Firmware Engineer",
        "employment",
        "NL",
        None,
    )
    .await;
    let f = |stage: Option<&str>, country: Option<&str>, q: Option<&str>| product::Filters {
        stage: stage.map(Into::into),
        country: country.map(Into::into),
        q: q.map(Into::into),
        ..Default::default()
    };
    assert_eq!(
        ranked_titles(&db.pool, &f(Some("Internship"), None, None)).await,
        vec!["Firmware Engineering Intern"]
    );
    assert_eq!(
        ranked_titles(&db.pool, &f(None, Some("nl"), None)).await,
        vec!["Embedded Firmware Engineer"]
    );
    assert_eq!(
        ranked_titles(&db.pool, &f(None, None, Some("other chips"))).await,
        vec!["Embedded Firmware Engineer"]
    );
    assert!(
        ranked_titles(&db.pool, &f(Some("Internship"), Some("NL"), None))
            .await
            .is_empty()
    );
    let home = product::dashboard_with(&db.pool, &f(None, Some("NL"), None))
        .await
        .unwrap();
    assert_eq!(
        (home["matches"].as_u64(), home["total_ranked"].as_u64()),
        (Some(1), Some(2))
    );
    assert_eq!(
        home["filter_options"]["countries"],
        serde_json::json!(["DE", "NL"])
    );
    db.cleanup().await;
}

#[tokio::test]
async fn notification_preferences_have_safe_defaults_and_refuse_nonsense() {
    let db = require_db!();
    setup(&db.pool).await;
    let defaults = preferences::notification_preferences(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        (
            defaults.min_priority,
            defaults.digest_hour,
            defaults.time_zone.as_str()
        ),
        (60, 8, "Europe/Berlin")
    );
    assert_eq!(defaults.reminder_days, vec![7, 3, 1]);
    let set = |p: preferences::NotificationPreferences| {
        preferences::set_notification_preferences(&db.pool, p)
    };
    for bad in [
        preferences::NotificationPreferences {
            time_zone: "Mars/Olympus".into(),
            ..defaults.clone()
        },
        preferences::NotificationPreferences {
            digest_hour: 24,
            ..defaults.clone()
        },
        preferences::NotificationPreferences {
            min_priority: 101,
            ..defaults.clone()
        },
        preferences::NotificationPreferences {
            reminder_days: vec![90],
            ..defaults.clone()
        },
        preferences::NotificationPreferences {
            reminder_days: vec![1, 2, 3, 4, 5, 6],
            ..defaults.clone()
        },
    ] {
        assert!(set(bad).await.is_err());
    }
    let saved = set(preferences::NotificationPreferences {
        time_zone: " America/Toronto ".into(),
        reminder_days: vec![1, 14, 1],
        ..defaults.clone()
    })
    .await
    .unwrap();
    assert_eq!(
        (saved.time_zone.as_str(), saved.reminder_days.clone()),
        ("America/Toronto", vec![14, 1])
    );
    assert_eq!(
        preferences::notification_preferences(&db.pool)
            .await
            .unwrap(),
        saved
    );
    db.cleanup().await;
}

async fn notifications(pool: &Db, kind: &str) -> Vec<serde_json::Value> {
    sqlx::query_scalar("select payload from notifications where kind = $1 order by created_at")
        .bind(kind)
        .fetch_all(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn the_new_match_digest_is_queued_once_a_day_without_dismissed_or_old_matches() {
    let db = require_db!();
    setup(&db.pool).await;
    let fresh = opportunity(
        &db.pool,
        "Example Silicon",
        "Firmware Engineering Intern",
        "internship",
        "DE",
        Some(20.0),
    )
    .await;
    let dismissed = opportunity(
        &db.pool,
        "Example Silicon",
        "Embedded Firmware Engineer",
        "employment",
        "DE",
        None,
    )
    .await;
    let old = opportunity(
        &db.pool,
        "Old Chips",
        "Firmware Engineer, Bootloaders",
        "employment",
        "DE",
        None,
    )
    .await;
    sqlx::query("update opportunities set first_seen_at = now() - interval '3 days' where id = $1")
        .bind(old)
        .execute(&db.pool)
        .await
        .unwrap();
    preferences::dismiss(&db.pool, dismissed).await.unwrap();
    let mut prefs = preferences::notification_preferences(&db.pool)
        .await
        .unwrap();
    prefs.digest_hour = 0;
    prefs.min_priority = 0;
    preferences::set_notification_preferences(&db.pool, prefs.clone())
        .await
        .unwrap();

    assert_eq!(
        alerts::queue_new_match_digest(&db.pool).await.unwrap(),
        Some(1)
    );
    for _ in 0..3 {
        assert_eq!(
            alerts::queue_new_match_digest(&db.pool).await.unwrap(),
            None,
            "once a day, however many ticks"
        );
    }
    let digests = notifications(&db.pool, "new_matches").await;
    assert_eq!(digests.len(), 1);
    let items = digests[0]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["opportunity_id"], serde_json::json!(fresh));
    assert_eq!(items[0]["stage"], "Internship");
    assert!(items[0]["deadline"].as_str().is_some());

    // Switched off: nothing is evaluated or queued.
    sqlx::query("update notification_preferences set last_digest_at = null")
        .execute(&db.pool)
        .await
        .unwrap();
    preferences::set_notification_preferences(
        &db.pool,
        preferences::NotificationPreferences {
            new_match_digest: false,
            ..prefs
        },
    )
    .await
    .unwrap();
    assert_eq!(
        alerts::queue_new_match_digest(&db.pool).await.unwrap(),
        None
    );
    assert_eq!(notifications(&db.pool, "new_matches").await.len(), 1);
    db.cleanup().await;
}

#[tokio::test]
async fn deadline_reminders_follow_stated_deadlines_of_chosen_opportunities_only() {
    let db = require_db!();
    setup(&db.pool).await;
    let saved = opportunity(
        &db.pool,
        "Example Institute",
        "Masterarbeit: RISC-V caches",
        "master_thesis",
        "DE",
        Some(2.5),
    )
    .await;
    let unsaved = opportunity(
        &db.pool,
        "Example Institute",
        "Masterarbeit: DRAM scheduling",
        "master_thesis",
        "DE",
        Some(2.5),
    )
    .await;
    let no_deadline = opportunity(
        &db.pool,
        "Example Institute",
        "Masterarbeit: CXL",
        "master_thesis",
        "DE",
        None,
    )
    .await;
    let applied = opportunity(
        &db.pool,
        "Example Silicon",
        "Firmware Engineering Intern",
        "internship",
        "DE",
        Some(1.5),
    )
    .await;
    let far = opportunity(
        &db.pool,
        "Example Silicon",
        "Firmware Intern, Platforms",
        "internship",
        "DE",
        Some(30.0),
    )
    .await;
    for id in [saved, no_deadline, applied, far] {
        preferences::set_mark(&db.pool, id, "saved", true)
            .await
            .unwrap();
    }
    let _ = unsaved;
    sqlx::query("insert into applications (owner_id, opportunity_id, status) select id, $1, 'submitted' from owners limit 1")
        .bind(applied).execute(&db.pool).await.unwrap();

    assert_eq!(alerts::queue_deadline_reminders(&db.pool).await.unwrap(), 1);
    assert_eq!(
        alerts::queue_deadline_reminders(&db.pool).await.unwrap(),
        0,
        "the 3-day reminder is not repeated"
    );
    let reminders = notifications(&db.pool, "deadline_reminder").await;
    assert_eq!(reminders.len(), 1);
    assert_eq!(reminders[0]["opportunity_id"], serde_json::json!(saved));
    assert_eq!(reminders[0]["days_left"], 3);
    let key: String =
        sqlx::query_scalar("select dedupe_key from notifications where kind = 'deadline_reminder'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        key,
        format!(
            "deadline:{saved}:{}:3",
            reminders[0]["deadline"].as_str().unwrap()
        )
    );

    // A day later it crosses the 1-day threshold: one more, and dismissing it stops the rest.
    sqlx::query("update opportunities set deadline_at = now() + interval '20 hours' where id = $1")
        .bind(saved)
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(alerts::queue_deadline_reminders(&db.pool).await.unwrap(), 1);
    preferences::dismiss(&db.pool, saved).await.unwrap();
    sqlx::query("delete from notifications where dedupe_key like $1")
        .bind(format!("deadline:{saved}:%:1"))
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(alerts::queue_deadline_reminders(&db.pool).await.unwrap(), 0);
    db.cleanup().await;
}

#[tokio::test]
async fn a_deadline_moved_to_another_date_gets_its_own_reminder_once() {
    let db = require_db!();
    setup(&db.pool).await;
    let thesis = opportunity(
        &db.pool,
        "Example Institute",
        "Masterarbeit: RISC-V caches",
        "master_thesis",
        "DE",
        Some(2.5),
    )
    .await;
    preferences::set_mark(&db.pool, thesis, "saved", true)
        .await
        .unwrap();
    assert_eq!(alerts::queue_deadline_reminders(&db.pool).await.unwrap(), 1);
    // The institute brings the deadline forward a day; it is still inside the 3-day window.
    sqlx::query(
        "update opportunities set deadline_at = deadline_at - interval '1 day' where id = $1",
    )
    .bind(thesis)
    .execute(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        alerts::queue_deadline_reminders(&db.pool).await.unwrap(),
        1,
        "the new date is announced"
    );
    assert_eq!(
        alerts::queue_deadline_reminders(&db.pool).await.unwrap(),
        0,
        "and only once"
    );
    let reminders = notifications(&db.pool, "deadline_reminder").await;
    assert_eq!(reminders.len(), 2);
    assert_ne!(reminders[0]["deadline"], reminders[1]["deadline"]);
    assert_eq!(reminders[1]["days_left"], 2);
    db.cleanup().await;
}

#[tokio::test]
async fn reminders_count_only_the_owners_own_applications() {
    let db = require_db!();
    setup(&db.pool).await;
    let mine = opportunity(
        &db.pool,
        "Example Institute",
        "Masterarbeit: RISC-V caches",
        "master_thesis",
        "DE",
        Some(2.5),
    )
    .await;
    let theirs = opportunity(
        &db.pool,
        "Example Institute",
        "Masterarbeit: DRAM scheduling",
        "master_thesis",
        "DE",
        Some(2.5),
    )
    .await;
    preferences::set_mark(&db.pool, mine, "saved", true)
        .await
        .unwrap();
    // Another owner applied to the one this owner saved, and is preparing the one this owner ignored.
    let other: Uuid =
        sqlx::query_scalar("insert into owners (handle) values ('other') returning id")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    for (opportunity, status) in [(mine, "submitted"), (theirs, "preparing")] {
        sqlx::query(
            "insert into applications (owner_id, opportunity_id, status) values ($1, $2, $3)",
        )
        .bind(other)
        .bind(opportunity)
        .bind(status)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    assert_eq!(alerts::queue_deadline_reminders(&db.pool).await.unwrap(), 1);
    let reminders = notifications(&db.pool, "deadline_reminder").await;
    assert_eq!(reminders[0]["opportunity_id"], serde_json::json!(mine));
    db.cleanup().await;
}

#[tokio::test]
async fn a_visit_returns_the_one_before_it() {
    let db = require_db!();
    setup(&db.pool).await;
    assert_eq!(preferences::record_visit(&db.pool).await.unwrap(), None);
    let first = preferences::last_visit(&db.pool)
        .await
        .unwrap()
        .expect("recorded");
    assert_eq!(
        preferences::record_visit(&db.pool).await.unwrap(),
        Some(first)
    );
    db.cleanup().await;
}

#[tokio::test]
async fn a_workday_application_is_handed_to_the_owner_once_and_tracked_only_by_hand() {
    let db = require_db!();
    setup(&db.pool).await;
    let id = opportunity(
        &db.pool,
        "Example Tenant",
        "Embedded Software Intern",
        "internship",
        "DE",
        None,
    )
    .await;
    let workday: Uuid = sqlx::query_scalar("insert into sources (key, display_name, kind, endpoint) values ('example-workday', 'Example Workday', 'workday', '{\"kind\":\"workday\",\"tenant\":\"example\",\"site\":\"careers\"}') returning id")
        .fetch_one(&db.pool).await.unwrap();
    // The only open posting is on Workday, which has no supported form path.
    sqlx::query("update postings set source_id = $2, url = 'https://example.wd3.myworkdayjobs.com/careers/job/1' where opportunity_id = $1")
        .bind(id).bind(workday).execute(&db.pool).await.unwrap();
    let app = product::enqueue(&db.pool, id).await.unwrap();
    product::process_application(&db.pool, app).await.unwrap();
    product::process_application(&db.pool, app).await.unwrap();

    let (status, reason): (String, Option<String>) =
        sqlx::query_as("select status, route_reason from applications where id = $1")
            .bind(app)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(status, "unsupported");
    assert_eq!(
        reason.as_deref(),
        Some("Complete this application on the employer website.")
    );
    let count = |sql: &'static str| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .bind(app)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(count("select count(*) from notifications where application_id = $1 and kind = 'manual_required'").await, 1);
    assert_eq!(
        count("select count(*) from application_attempts where application_id = $1").await,
        0,
        "nothing is sent to a Workday form"
    );

    // The owner applies on the employer's site and says so; it is counted as reported, never confirmed.
    let tracked =
        applications::update_tracking(&db.pool, app, "submitted", "Applied on the Workday portal")
            .await
            .unwrap();
    assert_eq!(tracked.status, "submitted");
    let home = product::dashboard(&db.pool).await.unwrap();
    assert_eq!(home["application_counts"]["reported"], 1);
    assert_eq!(home["application_counts"]["confirmed"], 0);
    assert_eq!(
        count("select count(*) from application_attempts where application_id = $1").await,
        0
    );
    db.cleanup().await;
}

#[tokio::test]
async fn a_job_found_on_linkedin_is_tracked_once_by_hand_with_its_cv_and_reminders() {
    let db = require_db!();
    setup(&db.pool).await;
    let job = |url: &str| product::ExternalJob {
        title: "Firmware Engineer".into(),
        employer: "Example Silicon GmbH".into(),
        url: url.into(),
        deadline_at: Some(chrono::Utc::now() + chrono::Duration::hours(60)),
    };
    for refused in [
        "http://www.linkedin.com/jobs/view/4012345678/",
        "javascript:alert(1)",
        "https://www.linkedin.com/feed/",
        "not a link",
    ] {
        assert!(
            product::track_external(&db.pool, job(refused))
                .await
                .is_err(),
            "{refused}"
        );
    }
    let (app, duplicate) = product::track_external(
        &db.pool,
        job("https://www.linkedin.com/jobs/view/firmware-engineer-at-example-4012345678/?refId=abc&trackingId=xyz#top"),
    )
    .await
    .unwrap();
    assert!(!duplicate);
    // The same job reached through search, with other tracking parameters, is the same application.
    let (again, duplicate) = product::track_external(
        &db.pool,
        job("https://de.linkedin.com/jobs/search/?currentJobId=4012345678&keywords=firmware"),
    )
    .await
    .unwrap();
    assert_eq!((again, duplicate), (app, true));

    let (status, reason, cv, url, attempts, mails): (String, String, Option<Uuid>, String, i64, i64) = sqlx::query_as(
        "select a.status, a.route_reason, a.cv_document_id, p.url,
                (select count(*) from application_attempts t where t.application_id = a.id),
                (select count(*) from notifications n where n.application_id = a.id)
         from applications a join postings p on p.opportunity_id = a.opportunity_id where a.id = $1",
    )
    .bind(app)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(status, "unsupported");
    assert!(reason.contains("www.linkedin.com"), "{reason}");
    assert!(cv.is_some(), "the CV version in use is recorded");
    assert_eq!(url, "https://www.linkedin.com/jobs/view/4012345678/");
    assert_eq!(
        (attempts, mails),
        (0, 0),
        "nothing is sent and the owner is not emailed about their own entry"
    );
    let source: (bool, String) = sqlx::query_as(
        "select enabled, support from sources where key = 'owner-entered-www.linkedin.com'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(source, (false, "manual_link".into()), "never scanned");

    // A manual-completion application still gets its deadline reminder until the owner applies.
    assert_eq!(alerts::queue_deadline_reminders(&db.pool).await.unwrap(), 1);
    applications::update_tracking(&db.pool, app, "submitted", "Applied with Easy Apply myself")
        .await
        .unwrap();
    let home = product::dashboard(&db.pool).await.unwrap();
    assert_eq!(home["application_counts"]["reported"], 1);
    assert_eq!(home["application_counts"]["confirmed"], 0);
    // Tracking it again after applying changes nothing.
    let (_, duplicate) = product::track_external(
        &db.pool,
        job("https://www.linkedin.com/jobs/view/4012345678"),
    )
    .await
    .unwrap();
    assert!(duplicate);
    let status: String = sqlx::query_scalar("select status from applications where id = $1")
        .bind(app)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(status, "submitted");
    db.cleanup().await;
}

#[tokio::test]
async fn keyword_rules_hide_matching_titles_or_descriptions_and_refuse_oversized_values() {
    let db = require_db!();
    setup(&db.pool).await;
    for (title, kind) in [
        ("Firmware Engineering Intern", "internship"),
        ("Embedded Software Engineer", "employment"),
    ] {
        opportunity(&db.pool, "Example Silicon", title, kind, "DE", None).await;
    }
    let all = product::Filters::default();
    let rule = |kind: &str, value: &str| profiles::NewRule {
        kind: kind.into(),
        value: value.into(),
        profile_id: None,
        opportunity_id: None,
        note: Some("Added in the workspace".into()),
    };
    assert_eq!(ranked_titles(&db.pool, &all).await.len(), 2);

    let by_title = profiles::create_rule(&db.pool, rule("title_term", "intern"))
        .await
        .unwrap();
    assert_eq!(
        ranked_titles(&db.pool, &all).await,
        ["Embedded Software Engineer"]
    );
    let by_description = profiles::create_rule(&db.pool, rule("description_term", "bootloader"))
        .await
        .unwrap();
    assert!(ranked_titles(&db.pool, &all).await.is_empty());

    assert!(
        profiles::create_rule(&db.pool, rule("title_term", &"x".repeat(201)))
            .await
            .is_err()
    );
    assert!(profiles::create_rule(&db.pool, rule("title_term", "   "))
        .await
        .is_err());

    profiles::delete_rule(&db.pool, by_description.id)
        .await
        .unwrap();
    profiles::delete_rule(&db.pool, by_title.id).await.unwrap();
    assert_eq!(ranked_titles(&db.pool, &all).await.len(), 2);
    db.cleanup().await;
}
