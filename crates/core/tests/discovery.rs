//! PostgreSQL integration tests for the discovery pipeline. Each test runs against its own freshly migrated database so the disappearance and deduplication assertions cannot be contaminated by another test's rows.

use std::time::Duration;

use chrono::Utc;
use opportunity_core::ingest::ingest;
use opportunity_core::matching::{self, MatchQuery};
use opportunity_core::query::{self, OpportunityQuery};
use opportunity_core::scheduler;
use opportunity_core::sources::{self, NewSource, Source, SourceUpdate};
use opportunity_core::Db;
use opportunity_core::{digest, profiles};
use opportunity_domain::{
    AdvertisedState, FetchedPosting, Profile, RemotePolicy, RunOutcome, ScanResult,
    SeniorityPolicy, SourceEndpoint, SourceRunStats,
};
use sqlx::{Connection, Executor, PgConnection};
use uuid::Uuid;

struct TestDb {
    pool: Db,
    admin_url: String,
    name: String,
    url: String,
}

impl TestDb {
    /// Returns `None` when no database is configured, so the suite is skippable on a machine without PostgreSQL. The session gate records whether these tests actually ran.
    async fn create() -> Option<Self> {
        let base = std::env::var("DATABASE_URL").ok()?;
        let (prefix, _) = base.rsplit_once('/')?;
        let admin_url = format!("{prefix}/postgres");
        let name = format!("opportunity_test_{}", Uuid::new_v4().simple());
        let mut admin = PgConnection::connect(&admin_url)
            .await
            .expect("connect to the maintenance database");
        admin
            .execute(format!(r#"create database "{name}""#).as_str())
            .await
            .expect("create the test database");
        let url = format!("{prefix}/{name}");
        let pool = opportunity_core::connect(&url)
            .await
            .expect("connect to the test database");
        opportunity_core::migrate(&pool)
            .await
            .expect("apply migrations to a fresh database");
        Some(Self {
            pool,
            admin_url,
            name,
            url,
        })
    }

    async fn cleanup(self) {
        let Self {
            pool,
            admin_url,
            name,
            url: _,
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

fn posting(native_id: &str, title: &str, location: Option<&str>) -> FetchedPosting {
    FetchedPosting {
        native_id: native_id.to_owned(),
        url: format!("https://example.org/jobs/{native_id}"),
        title: title.to_owned(),
        description_html: Some(format!(
            "<p>Work on {title} with memory systems and firmware.</p>"
        )),
        location_raw: location.map(str::to_owned),
        country_code: location.map(|_| "DE".to_owned()),
        city: location.map(|_| "Munich".to_owned()),
        remote_raw: None,
        department: Some("Platform".to_owned()),
        employment_type_raw: None,
        posted_at: Some(Utc::now()),
        supervisor: None,
        contact: None,
        deadline_at: None,
        advertised_state: None,
    }
}

fn scan_of(outcome: RunOutcome, postings: Vec<FetchedPosting>) -> ScanResult {
    let records_seen = postings.len() as i32;
    ScanResult {
        outcome,
        postings,
        stats: SourceRunStats {
            pages_fetched: 1,
            records_seen,
            total_reported: None,
        },
        diagnostic: None,
        raw_body: None,
        raw_url: None,
    }
}

async fn register(pool: &Db, key: &str, organization: &str, endpoint: SourceEndpoint) -> Source {
    sources::create(
        pool,
        NewSource {
            key: key.to_owned(),
            display_name: organization.to_owned(),
            organization: organization.to_owned(),
            endpoint,
            enabled: true,
            refresh_hours: 6,
            tags: Vec::new(),
        },
    )
    .await
    .expect("register the test source")
}

async fn open_postings(pool: &Db, source_id: Uuid) -> i64 {
    sqlx::query_scalar("select count(*) from postings where source_id = $1 and closed_at is null")
        .bind(source_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn migrations_apply_to_a_fresh_database_and_are_idempotent() {
    let db = require_db!();
    let tables: Vec<String> = sqlx::query_scalar(
        "select table_name from information_schema.tables where table_schema = 'public' order by table_name",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    for expected in [
        "opportunities",
        "opportunity_revisions",
        "organizations",
        "owners",
        "postings",
        "raw_documents",
        "source_runs",
        "sources",
    ] {
        assert!(
            tables.contains(&expected.to_owned()),
            "missing table {expected} in {tables:?}"
        );
    }
    // Running the migrator again on an up-to-date database must be a no-op rather than an error.
    opportunity_core::migrate(&db.pool)
        .await
        .expect("re-running migrations is a no-op");
    db.cleanup().await;
}

#[tokio::test]
async fn repeat_ingest_of_an_unchanged_board_is_idempotent() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "idem",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    let batch = vec![
        posting("1", "Firmware Engineer", Some("Munich, Germany")),
        posting("2", "Memory Systems Intern", Some("Munich, Germany")),
    ];

    let first = ingest(
        &db.pool,
        &source,
        scan_of(RunOutcome::OkComplete, batch.clone()),
    )
    .await
    .unwrap();
    assert_eq!(first.postings_created, 2);
    let second = ingest(
        &db.pool,
        &source,
        scan_of(RunOutcome::OkComplete, batch.clone()),
    )
    .await
    .unwrap();
    assert_eq!(second.postings_created, 0);
    assert_eq!(second.postings_updated, 2);
    assert_eq!(
        second.revisions_recorded, 0,
        "an unchanged posting must not generate a revision"
    );

    let postings: i64 = sqlx::query_scalar("select count(*) from postings")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let opportunities: i64 = sqlx::query_scalar("select count(*) from opportunities")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(postings, 2);
    assert_eq!(opportunities, 2);
    db.cleanup().await;
}

#[tokio::test]
async fn a_failed_or_partial_scan_never_closes_an_existing_posting() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "partial",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    let batch = vec![
        posting("1", "Firmware Engineer", Some("Munich, Germany")),
        posting("2", "Memory Systems Intern", Some("Munich, Germany")),
    ];
    ingest(&db.pool, &source, scan_of(RunOutcome::OkComplete, batch))
        .await
        .unwrap();
    assert_eq!(open_postings(&db.pool, source.id).await, 2);

    // Every non-complete outcome, including a partial sweep that genuinely saw one of the two postings, must leave both open.
    for outcome in [
        RunOutcome::Partial,
        RunOutcome::HttpError,
        RunOutcome::RateLimited,
        RunOutcome::AccessDenied,
        RunOutcome::SchemaError,
        RunOutcome::NotModified,
    ] {
        let observed = if outcome == RunOutcome::Partial {
            vec![posting("1", "Firmware Engineer", Some("Munich, Germany"))]
        } else {
            Vec::new()
        };
        let summary = ingest(&db.pool, &source, scan_of(outcome, observed))
            .await
            .unwrap();
        assert!(
            !summary.complete,
            "{outcome} must not be recorded as a complete sweep"
        );
        assert_eq!(
            summary.postings_marked_missing, 0,
            "{outcome} must not touch the missing streak"
        );
        assert_eq!(
            summary.postings_closed, 0,
            "{outcome} must not close a posting"
        );
        assert_eq!(
            open_postings(&db.pool, source.id).await,
            2,
            "{outcome} left a posting closed"
        );
    }

    let streaks: Vec<i32> =
        sqlx::query_scalar("select missing_streak from postings order by native_id")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(streaks, vec![0, 0]);
    db.cleanup().await;
}

#[tokio::test]
async fn only_repeated_complete_sweeps_close_a_vanished_posting() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "close",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    let both = vec![
        posting("1", "Firmware Engineer", Some("Munich, Germany")),
        posting("2", "Memory Systems Intern", Some("Munich, Germany")),
    ];
    ingest(&db.pool, &source, scan_of(RunOutcome::OkComplete, both))
        .await
        .unwrap();

    let only_first = vec![posting("1", "Firmware Engineer", Some("Munich, Germany"))];
    let first_miss = ingest(
        &db.pool,
        &source,
        scan_of(RunOutcome::OkComplete, only_first.clone()),
    )
    .await
    .unwrap();
    assert_eq!(first_miss.postings_marked_missing, 1);
    assert_eq!(
        first_miss.postings_closed, 0,
        "one complete sweep is not enough to close a posting"
    );
    assert_eq!(open_postings(&db.pool, source.id).await, 2);

    let second_miss = ingest(
        &db.pool,
        &source,
        scan_of(RunOutcome::OkComplete, only_first),
    )
    .await
    .unwrap();
    assert_eq!(second_miss.postings_closed, 1);
    assert_eq!(open_postings(&db.pool, source.id).await, 1);

    // The opportunity behind the closed posting closes with it, and a closed posting that reappears is reopened.
    let closed_opportunities: i64 =
        sqlx::query_scalar("select count(*) from opportunities where closed_at is not null")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(closed_opportunities, 1);

    let both_again = vec![
        posting("1", "Firmware Engineer", Some("Munich, Germany")),
        posting("2", "Memory Systems Intern", Some("Munich, Germany")),
    ];
    ingest(
        &db.pool,
        &source,
        scan_of(RunOutcome::OkComplete, both_again),
    )
    .await
    .unwrap();
    assert_eq!(open_postings(&db.pool, source.id).await, 2);
    let still_closed: i64 =
        sqlx::query_scalar("select count(*) from opportunities where closed_at is not null")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        still_closed, 0,
        "a posting that came back must reopen its opportunity"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn a_verified_empty_board_closes_postings_but_an_errored_one_does_not() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "empty",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    ingest(
        &db.pool,
        &source,
        scan_of(
            RunOutcome::OkComplete,
            vec![posting("1", "Firmware Engineer", Some("Munich, Germany"))],
        ),
    )
    .await
    .unwrap();

    ingest(
        &db.pool,
        &source,
        scan_of(RunOutcome::HttpError, Vec::new()),
    )
    .await
    .unwrap();
    assert_eq!(
        open_postings(&db.pool, source.id).await,
        1,
        "a 5xx is not evidence that the board is empty"
    );

    ingest(
        &db.pool,
        &source,
        scan_of(RunOutcome::OkVerifiedEmpty, Vec::new()),
    )
    .await
    .unwrap();
    ingest(
        &db.pool,
        &source,
        scan_of(RunOutcome::OkVerifiedEmpty, Vec::new()),
    )
    .await
    .unwrap();
    assert_eq!(
        open_postings(&db.pool, source.id).await,
        0,
        "two verified-empty sweeps do close the posting"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn the_same_role_on_two_boards_becomes_one_opportunity_with_both_postings_intact() {
    let db = require_db!();
    let greenhouse = register(
        &db.pool,
        "dup-gh",
        "Example Silicon GmbH",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    let lever = register(
        &db.pool,
        "dup-lv",
        "EXAMPLE SILICON, Inc.",
        SourceEndpoint::Lever {
            site_id: "example".into(),
        },
    )
    .await;

    ingest(
        &db.pool,
        &greenhouse,
        scan_of(
            RunOutcome::OkComplete,
            vec![posting(
                "gh-1",
                "Firmware Engineer (m/w/d)",
                Some("Munich, Germany"),
            )],
        ),
    )
    .await
    .unwrap();
    ingest(
        &db.pool,
        &lever,
        scan_of(
            RunOutcome::OkComplete,
            vec![posting("lv-1", "Firmware Engineer", Some("munich germany"))],
        ),
    )
    .await
    .unwrap();

    let opportunities: i64 = sqlx::query_scalar("select count(*) from opportunities")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let postings: i64 = sqlx::query_scalar("select count(*) from postings")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        opportunities, 1,
        "two spellings of one employer and title must converge on one opportunity"
    );
    assert_eq!(postings, 2, "both original postings must survive the merge");

    let detail = query::list_opportunities(
        &db.pool,
        &OpportunityQuery {
            q: None,
            country: None,
            city: None,
            kind: None,
            source: None,

            state: None,
            include_closed: false,
            page: None,
            per_page: None,
            sort: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(detail.items.len(), 1);
    assert_eq!(detail.items[0].posting_count, 2);
    assert_eq!(detail.items[0].source_names.len(), 2);

    // Unlinking is the documented way to reverse a wrong merge, and it must leave both postings in place.
    sqlx::query("update postings set opportunity_id = null where native_id = 'lv-1'")
        .execute(&db.pool)
        .await
        .unwrap();
    let postings_after: i64 = sqlx::query_scalar("select count(*) from postings")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(postings_after, 2);
    db.cleanup().await;
}

#[tokio::test]
async fn an_unrelated_employer_sharing_a_substring_is_not_merged() {
    let db = require_db!();
    let core = register(
        &db.pool,
        "core",
        "Core Systems",
        SourceEndpoint::Greenhouse {
            board_token: "core".into(),
        },
    )
    .await;
    let hardcore = register(
        &db.pool,
        "hardcore",
        "Hardcore Systems",
        SourceEndpoint::Greenhouse {
            board_token: "hardcore".into(),
        },
    )
    .await;
    ingest(
        &db.pool,
        &core,
        scan_of(
            RunOutcome::OkComplete,
            vec![posting("1", "Firmware Engineer", Some("Munich, Germany"))],
        ),
    )
    .await
    .unwrap();
    ingest(
        &db.pool,
        &hardcore,
        scan_of(
            RunOutcome::OkComplete,
            vec![posting("1", "Firmware Engineer", Some("Munich, Germany"))],
        ),
    )
    .await
    .unwrap();

    let organizations: i64 = sqlx::query_scalar("select count(*) from organizations")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let opportunities: i64 = sqlx::query_scalar("select count(*) from opportunities")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(organizations, 2);
    assert_eq!(opportunities, 2);
    db.cleanup().await;
}

#[tokio::test]
async fn a_duplicate_native_identifier_within_one_sweep_is_stored_once() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "dupid",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    let repeated = vec![
        posting("1", "Firmware Engineer", Some("Munich, Germany")),
        posting("1", "Firmware Engineer", Some("Munich, Germany")),
        posting("2", "Verification Engineer", Some("Munich, Germany")),
    ];
    let summary = ingest(&db.pool, &source, scan_of(RunOutcome::OkComplete, repeated))
        .await
        .unwrap();
    assert_eq!(summary.postings_created, 2);
    let postings: i64 = sqlx::query_scalar("select count(*) from postings")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(postings, 2);
    db.cleanup().await;
}

#[tokio::test]
async fn a_changed_description_records_a_revision() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "rev",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    ingest(
        &db.pool,
        &source,
        scan_of(
            RunOutcome::OkComplete,
            vec![posting("1", "Firmware Engineer", Some("Munich, Germany"))],
        ),
    )
    .await
    .unwrap();

    let mut edited = posting("1", "Firmware Engineer", Some("Munich, Germany"));
    edited.description_html =
        Some("<p>Revised scope: now also covers secure boot and the memory controller.</p>".into());
    let summary = ingest(
        &db.pool,
        &source,
        scan_of(RunOutcome::OkComplete, vec![edited]),
    )
    .await
    .unwrap();
    assert_eq!(summary.revisions_recorded, 1);
    let revisions: i64 = sqlx::query_scalar("select count(*) from opportunity_revisions")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(revisions, 1);
    db.cleanup().await;
}

#[tokio::test]
async fn a_posting_without_a_description_is_stored_and_flagged_title_only() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "bare",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    let mut bare = posting(
        "1",
        "Electrical Design Engineer",
        Some("Karlsruhe, Germany"),
    );
    bare.description_html = None;
    ingest(
        &db.pool,
        &source,
        scan_of(RunOutcome::OkComplete, vec![bare]),
    )
    .await
    .unwrap();

    let (title, title_only): (String, bool) =
        sqlx::query_as("select title, title_only from postings")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(title, "Electrical Design Engineer");
    assert!(
        title_only,
        "a row with no description stays valid and is flagged for ranking"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn full_text_search_finds_german_and_english_terms() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "search",
        "Example Silicon",
        SourceEndpoint::Lever {
            site_id: "example".into(),
        },
    )
    .await;
    let mut thesis = posting(
        "1",
        "Masterarbeit: Speicherhierarchien",
        Some("Karlsruhe, Germany"),
    );
    thesis.description_html = Some(
        "<p>Abschlussarbeit zu Seitenmigration und Speicherhierarchien auf RISC-V.</p>".into(),
    );
    let english = posting("2", "Memory Systems Engineer", Some("Munich, Germany"));
    ingest(
        &db.pool,
        &source,
        scan_of(RunOutcome::OkComplete, vec![thesis, english]),
    )
    .await
    .unwrap();

    let search = |q: &str| {
        let pool = db.pool.clone();
        let q = q.to_owned();
        async move {
            query::list_opportunities(
                &pool,
                &OpportunityQuery {
                    q: Some(q),
                    country: None,
                    city: None,
                    kind: None,
                    source: None,

                    state: None,
                    include_closed: false,
                    page: None,
                    per_page: None,
                    sort: Some("relevance".into()),
                },
            )
            .await
            .unwrap()
        }
    };
    assert_eq!(search("Speicherhierarchien").await.total, 1);
    assert_eq!(search("memory").await.total, 1);
    assert_eq!(search("Seitenmigration").await.total, 1);
    assert_eq!(search("quantum").await.total, 0);

    // The German thesis must be classified as a thesis, not filtered out as a non-software title.
    let thesis_only = query::list_opportunities(
        &db.pool,
        &OpportunityQuery {
            q: None,
            country: None,
            city: None,
            kind: Some("master_thesis".into()),
            source: None,

            state: None,
            include_closed: false,
            page: None,
            per_page: None,
            sort: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(thesis_only.total, 1);
    assert_eq!(thesis_only.items[0].kind, "master_thesis");
    db.cleanup().await;
}

#[tokio::test]
async fn coverage_reports_an_unread_source_as_not_configured_rather_than_empty() {
    let db = require_db!();
    let scanned = register(
        &db.pool,
        "scanned",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    register(
        &db.pool,
        "never",
        "Untouched Labs",
        SourceEndpoint::Ashby {
            board_slug: "untouched".into(),
        },
    )
    .await;
    register(
        &db.pool,
        "manual",
        "Apple",
        SourceEndpoint::Manual {
            url: "https://jobs.apple.com/".into(),
        },
    )
    .await;
    ingest(
        &db.pool,
        &scanned,
        scan_of(
            RunOutcome::OkComplete,
            vec![posting("1", "Firmware Engineer", Some("Munich, Germany"))],
        ),
    )
    .await
    .unwrap();

    let rows = query::coverage(&db.pool).await.unwrap();
    let by_key = |key: &str| {
        rows.iter()
            .find(|r| r.key == key)
            .cloned()
            .expect("source present in coverage")
    };
    assert_eq!(by_key("scanned").support, "verified_live");
    assert_eq!(by_key("scanned").open_postings, 1);
    assert_eq!(by_key("never").support, "not_configured");
    assert_eq!(by_key("never").open_postings, 0);
    assert!(by_key("never").last_attempt_at.is_none());
    assert_eq!(
        by_key("manual").support,
        "manual_link",
        "a manual watch entry never claims discovered jobs"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn a_blocked_source_is_labeled_blocked_and_keeps_its_previous_records() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "blocked",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    ingest(
        &db.pool,
        &source,
        scan_of(
            RunOutcome::OkComplete,
            vec![posting("1", "Firmware Engineer", Some("Munich, Germany"))],
        ),
    )
    .await
    .unwrap();
    let mut denied = scan_of(RunOutcome::AccessDenied, Vec::new());
    denied.diagnostic = Some("greenhouse board responded 403 Forbidden".into());
    ingest(&db.pool, &source, denied).await.unwrap();

    let row = sources::get(&db.pool, source.id).await.unwrap();
    assert_eq!(row.support, "blocked");
    assert_eq!(row.health, "blocked");
    assert_eq!(row.last_outcome.as_deref(), Some("access_denied"));
    assert!(row.last_diagnostic.as_deref().unwrap().contains("403"));
    assert_eq!(
        open_postings(&db.pool, source.id).await,
        1,
        "a refusal does not delete what was already discovered"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn source_runs_record_the_completeness_verdict_of_every_attempt() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "runs",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    ingest(
        &db.pool,
        &source,
        scan_of(
            RunOutcome::OkComplete,
            vec![posting("1", "Firmware Engineer", Some("Munich, Germany"))],
        ),
    )
    .await
    .unwrap();
    let mut partial = scan_of(
        RunOutcome::Partial,
        vec![posting("1", "Firmware Engineer", Some("Munich, Germany"))],
    );
    partial.diagnostic = Some("board reported 9 jobs but the response carried 1".into());
    ingest(&db.pool, &source, partial).await.unwrap();

    let runs = query::source_runs(&db.pool, Some(source.id), 10)
        .await
        .unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].outcome, "partial");
    assert!(!runs[0].complete);
    assert!(runs[0]
        .diagnostic
        .as_deref()
        .unwrap()
        .contains("reported 9"));
    assert_eq!(runs[1].outcome, "ok_complete");
    assert!(runs[1].complete);
    db.cleanup().await;
}

#[tokio::test]
async fn registering_a_source_with_a_placeholder_identifier_is_rejected() {
    let db = require_db!();
    let result = sources::create(
        &db.pool,
        NewSource {
            key: "bad".into(),
            display_name: "Example".into(),
            organization: "Example".into(),
            endpoint: SourceEndpoint::Greenhouse {
                board_token: "REPLACE_WITH_VERIFIED_BOARD_TOKEN".into(),
            },
            enabled: true,
            refresh_hours: 6,
            tags: Vec::new(),
        },
    )
    .await;
    assert!(
        result.is_err(),
        "an unconfigured template identifier must not become a registered source"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn importing_the_seed_file_registers_only_validated_entries_and_is_repeatable() {
    let db = require_db!();
    let text = include_str!("../../../config/sources.seed.toml");
    let first = opportunity_core::config::import_sources(&db.pool, text)
        .await
        .unwrap();
    assert!(
        first.rejected.is_empty(),
        "verified seed entries must all validate: {:?}",
        first.rejected
    );
    assert!(!first.created.is_empty());

    let second = opportunity_core::config::import_sources(&db.pool, text)
        .await
        .unwrap();
    assert!(
        second.created.is_empty(),
        "a second import must not duplicate sources"
    );
    assert_eq!(second.skipped_existing.len(), first.created.len());

    let template = include_str!("../../../config/sources.example.toml");
    let report = opportunity_core::config::import_sources(&db.pool, template)
        .await
        .unwrap();
    assert!(
        report
            .rejected
            .iter()
            .all(|(key, _)| key.starts_with("example-")),
        "only the unconfigured template entries should be rejected: {:?}",
        report.rejected
    );
    assert_eq!(
        report.rejected.len(),
        5,
        "every placeholder template entry is rejected with a reason"
    );
    db.cleanup().await;
}

// ---------------------------------------------------------------------------
// Session 2: academic records, Workday pagination safety and the scheduler.
// ---------------------------------------------------------------------------

fn academic_posting(native_id: &str, title: &str, state: AdvertisedState) -> FetchedPosting {
    FetchedPosting {
        native_id: native_id.to_owned(),
        url: format!("https://example-institut.test/arbeiten/{native_id}"),
        title: title.to_owned(),
        description_html: Some(format!("<p>{title}</p>")),
        location_raw: Some("Karlsruhe, Deutschland".to_owned()),
        country_code: Some("DE".to_owned()),
        city: Some("Karlsruhe".to_owned()),
        remote_raw: None,
        department: Some("Lehrstuhl für Rechnerarchitektur".to_owned()),
        employment_type_raw: None,
        posted_at: None,
        supervisor: Some("Prof. Dr. Anna Beispiel".to_owned()),
        contact: None,
        deadline_at: Some(Utc::now() + chrono::Duration::days(30)),
        advertised_state: Some(state),
    }
}

#[tokio::test]
async fn an_academic_open_topic_is_persisted_as_a_topic_and_never_as_a_funded_vacancy() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "institut",
        "Example Institut",
        SourceEndpoint::AcademicHtml {
            url: "https://example-institut.test/abschlussarbeiten".into(),
            pattern: None,
            selectors: None,
        },
    )
    .await;

    let batch = vec![
        academic_posting(
            "thesis-1",
            "Masterarbeit: Seitenmigration",
            AdvertisedState::OpenTopic,
        ),
        academic_posting(
            "wimi-1",
            "Wissenschaftliche Mitarbeiterin Speichersysteme",
            AdvertisedState::Advertised,
        ),
        academic_posting(
            "person-1",
            "Forschungsinteressen von Prof. Dr. Anna Beispiel",
            AdvertisedState::PotentialSupervisor,
        ),
    ];
    ingest(&db.pool, &source, scan_of(RunOutcome::OkComplete, batch))
        .await
        .unwrap();

    let states: Vec<(String, String)> =
        sqlx::query_as("select native_id, advertised_state from postings order by native_id")
            .fetch_all(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        states,
        vec![
            ("person-1".to_string(), "potential_supervisor".to_string()),
            ("thesis-1".to_string(), "open_topic".to_string()),
            ("wimi-1".to_string(), "advertised".to_string()),
        ],
        "the adapter's own statement about each record must survive ingest unchanged"
    );

    // An ATS-style default must never overwrite what the academic adapter said.
    let advertised_opportunities: i64 = sqlx::query_scalar(
        "select count(*) from opportunities where advertised_state = 'advertised'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        advertised_opportunities, 1,
        "only the TV-L post is a funded vacancy"
    );

    let (supervisor, deadline): (Option<String>, Option<chrono::DateTime<Utc>>) =
        sqlx::query_as("select supervisor, deadline_at from opportunities where canonical_title like 'Masterarbeit%'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(supervisor.as_deref(), Some("Prof. Dr. Anna Beispiel"));
    assert!(deadline.is_some(), "a stated deadline must be persisted");
    db.cleanup().await;
}

#[tokio::test]
async fn an_academic_record_without_a_supervisor_or_deadline_stores_nulls_rather_than_defaults() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "institut-sparse",
        "Example Institut",
        SourceEndpoint::AcademicHtml {
            url: "https://example-institut.test/abschlussarbeiten".into(),
            pattern: Some("successfactors".into()),
            selectors: None,
        },
    )
    .await;
    let mut sparse = academic_posting(
        "thesis-2",
        "Masterarbeit: RISC-V Speicherhierarchien",
        AdvertisedState::OpenTopic,
    );
    sparse.supervisor = None;
    sparse.deadline_at = None;
    ingest(
        &db.pool,
        &source,
        scan_of(RunOutcome::OkComplete, vec![sparse]),
    )
    .await
    .unwrap();

    let (supervisor, deadline): (Option<String>, Option<chrono::DateTime<Utc>>) =
        sqlx::query_as("select supervisor, deadline_at from postings")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert!(
        supervisor.is_none(),
        "an unnamed supervisor must stay unknown"
    );
    assert!(deadline.is_none(), "an unstated deadline must stay unknown");
    db.cleanup().await;
}

#[tokio::test]
async fn a_partial_workday_sweep_ingests_what_it_saw_without_closing_what_it_missed() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "workday-tenant",
        "Example Chip",
        SourceEndpoint::Workday {
            tenant: "examplechip".into(),
            instance: "wd3".into(),
            site: "careers".into(),
        },
    )
    .await;

    let full = vec![
        posting(
            "R-100001",
            "Senior Firmware Engineer",
            Some("Munich, Germany"),
        ),
        posting(
            "R-100002",
            "Memory Subsystem Verification Engineer",
            Some("Munich, Germany"),
        ),
        posting(
            "R-100003",
            "Werkstudent SoC Integration",
            Some("Dresden, Germany"),
        ),
    ];
    ingest(&db.pool, &source, scan_of(RunOutcome::OkComplete, full))
        .await
        .unwrap();
    assert_eq!(open_postings(&db.pool, source.id).await, 3);

    // A blank middle page: the sweep saw one record and is known incomplete.
    let mut partial = scan_of(
        RunOutcome::Partial,
        vec![posting(
            "R-100001",
            "Senior Firmware Engineer",
            Some("Munich, Germany"),
        )],
    );
    partial.diagnostic =
        Some("page 1 came back empty while the tenant still reported more postings".into());
    let summary = ingest(&db.pool, &source, partial).await.unwrap();

    assert!(!summary.complete);
    assert_eq!(summary.postings_marked_missing, 0);
    assert_eq!(summary.postings_closed, 0);
    assert_eq!(
        open_postings(&db.pool, source.id).await,
        3,
        "a blank middle page must not close the two it did not see"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn the_scheduler_persists_its_state_so_a_restart_does_not_rescan_everything() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "sched",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;

    // A freshly enabled source is due at once.
    let due = scheduler::due_now(&db.pool).await.unwrap();
    assert_eq!(due.len(), 1, "a newly enabled source is due immediately");

    let next = scheduler::record_schedule(&db.pool, &source, RunOutcome::OkComplete)
        .await
        .unwrap();
    assert!(
        next > Utc::now() + chrono::Duration::hours(5),
        "a healthy scan schedules the next one a full cadence away"
    );
    assert!(
        scheduler::due_now(&db.pool).await.unwrap().is_empty(),
        "the source is no longer due"
    );

    // Reading the schedule back through a new query is what a restarted process does.
    let reloaded = sources::get(&db.pool, source.id).await.unwrap();
    assert_eq!(reloaded.consecutive_failures, 0);
    assert!(reloaded.backoff_until.is_none());
    assert!(reloaded.next_run_at.is_some());
    assert!(
        scheduler::due_now(&db.pool).await.unwrap().is_empty(),
        "a restart must not make a just-scanned source due again"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn repeated_failures_back_a_source_off_and_a_success_clears_it() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "flaky",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;

    let mut current = source.clone();
    let mut previous_wait = chrono::Duration::zero();
    for attempt in 1..=3 {
        scheduler::record_schedule(&db.pool, &current, RunOutcome::HttpError)
            .await
            .unwrap();
        current = sources::get(&db.pool, source.id).await.unwrap();
        assert_eq!(current.consecutive_failures, attempt);
        let backoff = current
            .backoff_until
            .expect("a failing source is put in backoff");
        let wait = backoff - Utc::now();
        assert!(
            wait > previous_wait,
            "backoff must grow: {wait} did not exceed {previous_wait}"
        );
        previous_wait = wait;
        assert!(
            scheduler::due_now(&db.pool).await.unwrap().is_empty(),
            "a source in backoff is not due"
        );
    }

    scheduler::record_schedule(&db.pool, &current, RunOutcome::OkComplete)
        .await
        .unwrap();
    let recovered = sources::get(&db.pool, source.id).await.unwrap();
    assert_eq!(
        recovered.consecutive_failures, 0,
        "one success clears the failure streak"
    );
    assert!(
        recovered.backoff_until.is_none(),
        "one success clears the backoff"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn a_disabled_source_is_never_due_and_re_enabling_makes_it_due_again() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "toggle",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    assert_eq!(scheduler::due_now(&db.pool).await.unwrap().len(), 1);

    sources::update(
        &db.pool,
        source.id,
        SourceUpdate {
            enabled: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        scheduler::due_now(&db.pool).await.unwrap().is_empty(),
        "a disabled source is never scanned"
    );

    sources::update(
        &db.pool,
        source.id,
        SourceUpdate {
            enabled: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        scheduler::due_now(&db.pool).await.unwrap().len(),
        1,
        "re-enabling makes it due again"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn coverage_separates_freshness_from_what_the_adapter_has_proven() {
    let db = require_db!();
    let fresh = register(
        &db.pool,
        "fresh",
        "Fresh Labs",
        SourceEndpoint::Greenhouse {
            board_token: "fresh".into(),
        },
    )
    .await;
    let stale = register(
        &db.pool,
        "stale",
        "Stale Labs",
        SourceEndpoint::Greenhouse {
            board_token: "stale".into(),
        },
    )
    .await;
    register(
        &db.pool,
        "unchecked",
        "Unchecked Labs",
        SourceEndpoint::Ashby {
            board_slug: "unchecked".into(),
        },
    )
    .await;

    ingest(
        &db.pool,
        &fresh,
        scan_of(
            RunOutcome::OkComplete,
            vec![posting("1", "Firmware Engineer", Some("Munich, Germany"))],
        ),
    )
    .await
    .unwrap();
    ingest(
        &db.pool,
        &stale,
        scan_of(
            RunOutcome::OkComplete,
            vec![posting("1", "Firmware Engineer", Some("Munich, Germany"))],
        ),
    )
    .await
    .unwrap();
    // Push the second source's last success well past three of its own refresh cycles.
    sqlx::query(
        "update sources set last_success_at = now() - interval '40 hours' where key = 'stale'",
    )
    .execute(&db.pool)
    .await
    .unwrap();

    let rows = query::coverage(&db.pool).await.unwrap();
    let by_key = |key: &str| rows.iter().find(|r| r.key == key).cloned().unwrap();
    assert_eq!(by_key("fresh").freshness, "fresh");
    assert_eq!(by_key("stale").freshness, "stale");
    assert_eq!(by_key("unchecked").freshness, "never");
    // Staleness is independent of what the adapter proved: a source can be both verified live and out of date.
    assert_eq!(by_key("stale").support, "verified_live");
    assert_eq!(by_key("unchecked").support, "not_configured");
    db.cleanup().await;
}

#[tokio::test]
async fn stats_report_open_topics_separately_from_advertised_vacancies() {
    let db = require_db!();
    let institute = register(
        &db.pool,
        "topics",
        "Example Institut",
        SourceEndpoint::AcademicHtml {
            url: "https://example-institut.test/abschlussarbeiten".into(),
            pattern: None,
            selectors: None,
        },
    )
    .await;
    let company = register(
        &db.pool,
        "company",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;

    ingest(
        &db.pool,
        &institute,
        scan_of(
            RunOutcome::OkComplete,
            vec![academic_posting(
                "t1",
                "Masterarbeit: Seitenmigration",
                AdvertisedState::OpenTopic,
            )],
        ),
    )
    .await
    .unwrap();
    ingest(
        &db.pool,
        &company,
        scan_of(
            RunOutcome::OkComplete,
            vec![posting("1", "Firmware Engineer", Some("Munich, Germany"))],
        ),
    )
    .await
    .unwrap();

    let stats = query::stats(&db.pool).await.unwrap();
    assert_eq!(stats.opportunities_open, 2);
    assert_eq!(
        stats.open_topics, 1,
        "a thesis topic is counted as a topic, not as a vacancy"
    );
    assert!(
        stats.next_scheduled_run_at.is_some(),
        "enabled sources have a next run recorded"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn a_policy_refusal_labels_a_source_fixture_only_rather_than_blocked_by_the_employer() {
    let db = require_db!();
    let workday = register(
        &db.pool,
        "wd",
        "Example Chip",
        SourceEndpoint::Workday {
            tenant: "examplechip".into(),
            instance: "wd3".into(),
            site: "careers".into(),
        },
    )
    .await;
    assert_eq!(
        workday.support, "fixture_only",
        "an adapter that cannot yet run live starts as fixture only"
    );

    // The refusal comes from this project's own access policy, not from the employer, so it must not read as the source turning us away.
    let mut refused = scan_of(RunOutcome::AccessDenied, Vec::new());
    refused.diagnostic = Some("live Workday reads are off by default".into());
    ingest(&db.pool, &workday, refused).await.unwrap();

    let after = sources::get(&db.pool, workday.id).await.unwrap();
    assert_eq!(
        after.support, "fixture_only",
        "a policy refusal must not downgrade the label to blocked"
    );
    assert_eq!(after.last_outcome.as_deref(), Some("access_denied"));

    // Once the owner opts in and a real sweep succeeds, the label does advance.
    ingest(
        &db.pool,
        &workday,
        scan_of(
            RunOutcome::OkComplete,
            vec![posting("R-1", "Firmware Engineer", Some("Munich, Germany"))],
        ),
    )
    .await
    .unwrap();
    let verified = sources::get(&db.pool, workday.id).await.unwrap();
    assert_eq!(
        verified.support, "verified_live",
        "a genuine complete sweep earns verified live"
    );
    assert_eq!(verified.health, "healthy");
    db.cleanup().await;
}

#[tokio::test]
async fn a_manual_entry_keeps_its_label_through_any_attempt() {
    let db = require_db!();
    let manual = register(
        &db.pool,
        "manual2",
        "Apple",
        SourceEndpoint::Manual {
            url: "https://jobs.apple.com/".into(),
        },
    )
    .await;
    assert_eq!(manual.support, "manual_link");

    let mut refused = scan_of(RunOutcome::AccessDenied, Vec::new());
    refused.diagnostic = Some("manual watch entry: no adapter".into());
    ingest(&db.pool, &manual, refused).await.unwrap();

    let after = sources::get(&db.pool, manual.id).await.unwrap();
    assert_eq!(after.support, "manual_link");
    assert_eq!(
        after.health, "unknown",
        "a source nothing was asked of has no health verdict"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn two_positions_differing_only_after_a_gender_marker_do_not_merge() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "phd-lab",
        "Example Institute",
        SourceEndpoint::AcademicHtml {
            url: "https://jobs.example.test/search/".into(),
            pattern: Some("successfactors".into()),
            selectors: None,
        },
    )
    .await;

    // Two genuinely different PhD posts in one lab. A title normalizer that truncated at the first bracket would collapse them into one opportunity and hide a real opening.
    let a = posting(
        "1",
        "PhD Candidate (m/f/x) in the Cyber Threat Intelligence Lab - KriKom",
        Some("Potsdam, DE"),
    );
    let b = posting(
        "2",
        "PhD Candidate (m/f/x) in the Cyber Threat Intelligence Lab - ReDiKom",
        Some("Potsdam, DE"),
    );
    ingest(
        &db.pool,
        &source,
        scan_of(RunOutcome::OkComplete, vec![a, b]),
    )
    .await
    .unwrap();

    let opportunities: i64 = sqlx::query_scalar("select count(*) from opportunities")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let postings: i64 = sqlx::query_scalar("select count(*) from postings")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(postings, 2);
    assert_eq!(
        opportunities, 2,
        "two different positions must stay two opportunities"
    );

    // The genuine cross-source duplicate case must still merge, so the fix did not simply disable deduplication.
    let other = register(
        &db.pool,
        "phd-lab-mirror",
        "Example Institute",
        SourceEndpoint::Greenhouse {
            board_token: "mirror".into(),
        },
    )
    .await;
    let mirror = posting(
        "gh-1",
        "PhD Candidate in the Cyber Threat Intelligence Lab - KriKom",
        Some("Potsdam, DE"),
    );
    ingest(
        &db.pool,
        &other,
        scan_of(RunOutcome::OkComplete, vec![mirror]),
    )
    .await
    .unwrap();
    let after: i64 = sqlx::query_scalar("select count(*) from opportunities")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(after, 2, "the same position on a second board still merges");
    db.cleanup().await;
}

// ---------------------------------------------------------------------------
// Session 3: profiles, exclusion rules, matching, feedback and the digest.
// ---------------------------------------------------------------------------

async fn seeded_profile(
    pool: &Db,
    key: &str,
    terms: &[&str],
    countries: &[&str],
) -> profiles::StoredProfile {
    let mut profile = Profile::blank(key, key);
    profile.terms = terms.iter().map(|t| (*t).to_owned()).collect();
    profile.preferred_countries = countries.iter().map(|c| (*c).to_owned()).collect();
    profile.current_country = Some("DE".into());
    profile.relocation_willing = true;
    profiles::create(pool, profile)
        .await
        .expect("create the profile")
}

async fn ingest_one(
    pool: &Db,
    source: &Source,
    native_id: &str,
    title: &str,
    description: &str,
    country: Option<&str>,
) {
    let mut p = posting(native_id, title, country.map(|_| "Munich, Germany"));
    p.description_html = Some(format!("<p>{description}</p>"));
    p.country_code = country.map(str::to_owned);
    p.city = country.map(|_| "Munich".to_owned());
    ingest(pool, source, scan_of(RunOutcome::OkComplete, vec![p]))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_profile_survives_a_restart_with_every_field_intact() {
    let db = require_db!();
    let mut profile = Profile::blank("architecture", "Computer architecture");
    profile.terms = vec!["DRAM".into(), "CXL".into()];
    profile.job_kinds = vec!["employment".into(), "master_thesis".into()];
    profile.preferred_countries = vec!["DE".into(), "SE".into()];
    profile.preferred_cities = vec!["Munich".into()];
    profile.current_country = Some("DE".into());
    profile.relocation_willing = true;
    profile.remote_policy = RemotePolicy::OnSiteOrRelocate;
    profile.seniority_policy = SeniorityPolicy::Hard;
    profile.max_years_experience = Some(5);
    profile.education_level = Some("MSc".into());
    profile.work_authorization = vec!["DE".into()];
    profile.excluded_technologies = vec!["COBOL".into()];
    profile.weights.semantic = 3.5;
    let created = profiles::create(&db.pool, profile.clone()).await.unwrap();

    // Reading through a fresh pool is what a restarted process does.
    let reloaded = profiles::get(&db.pool, created.id).await.unwrap();
    assert_eq!(
        reloaded.profile, profile,
        "every field must survive the round trip"
    );
    assert_eq!(reloaded.profile.weights.semantic, 3.5);
    assert_eq!(
        reloaded.profile.remote_policy,
        RemotePolicy::OnSiteOrRelocate
    );
    assert_eq!(reloaded.profile.seniority_policy, SeniorityPolicy::Hard);

    // An edit persists too, and the whole profile is replaced rather than partially patched.
    let mut edited = reloaded.profile.clone();
    edited.terms = vec!["RISC-V".into()];
    edited.relocation_willing = false;
    profiles::update(&db.pool, created.id, edited.clone())
        .await
        .unwrap();
    assert_eq!(
        profiles::get(&db.pool, created.id).await.unwrap().profile,
        edited
    );
    db.cleanup().await;
}

#[tokio::test]
async fn the_four_starting_profiles_seed_once_and_carry_no_personal_data() {
    let db = require_db!();
    let created = profiles::seed_default_profiles(&db.pool).await.unwrap();
    assert_eq!(
        created.len(),
        4,
        "the contract asks for four tracks: {created:?}"
    );

    let all = profiles::list(&db.pool).await.unwrap();
    for stored in &all {
        assert!(
            !stored.profile.terms.is_empty(),
            "{} has no interests",
            stored.profile.key
        );
        // A shipped default must not carry anyone's location, authorization or education.
        assert!(
            stored.profile.current_country.is_none(),
            "{} leaked a location",
            stored.profile.key
        );
        assert!(
            stored.profile.work_authorization.is_empty(),
            "{} leaked work authorization",
            stored.profile.key
        );
        assert!(
            stored.profile.education_level.is_none(),
            "{} leaked an education level",
            stored.profile.key
        );
        assert!(
            stored.profile.preferred_cities.is_empty(),
            "{} leaked a city preference",
            stored.profile.key
        );
    }

    // Seeding again must not duplicate or overwrite an edited profile.
    let mut edited = all[0].profile.clone();
    edited.terms = vec!["edited by the owner".into()];
    profiles::update(&db.pool, all[0].id, edited).await.unwrap();
    assert!(profiles::seed_default_profiles(&db.pool)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(profiles::list(&db.pool).await.unwrap().len(), 4);
    assert_eq!(
        profiles::get(&db.pool, all[0].id)
            .await
            .unwrap()
            .profile
            .terms,
        vec!["edited by the owner"]
    );
    db.cleanup().await;
}

#[tokio::test]
async fn matching_ranks_real_stored_opportunities_and_explains_each_one() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "match",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    ingest_one(
        &db.pool,
        &source,
        "1",
        "Systems Software Engineer, Tiered Memory",
        "Design page migration policies across CXL-attached capacity.",
        Some("DE"),
    )
    .await;
    ingest_one(
        &db.pool,
        &source,
        "2",
        "Office Manager",
        "Keep the Munich office running smoothly.",
        Some("DE"),
    )
    .await;
    let profile = seeded_profile(&db.pool, "architecture", &["DRAM", "RISC-V"], &["DE"]).await;

    let page = matching::matches_for_profile(
        &db.pool,
        profile.id,
        &MatchQuery {
            q: None,
            limit: 25,
            include_excluded: false,
        },
        None,
    )
    .await
    .unwrap();

    assert_eq!(page.total_considered, 2);
    assert!(
        page.ranking.contains("lexical ranking only"),
        "ranking must be labelled honestly: {}",
        page.ranking
    );
    assert_eq!(
        page.items[0].opportunity.canonical_title,
        "Systems Software Engineer, Tiered Memory"
    );
    assert!(page.items[0].result.score > page.items[1].result.score);
    // The memory role matched without the posting ever using the word DRAM, and the explanation says which phrase did it.
    assert!(page.items[0]
        .result
        .matched_concepts
        .contains(&"memory systems".to_string()));
    let related = page.items[0]
        .result
        .factors
        .iter()
        .find(|f| f.name == "related wording")
        .unwrap();
    assert!(related
        .evidence
        .iter()
        .any(|e| e.contains("page migration") || e.contains("cxl")));
    db.cleanup().await;
}

#[tokio::test]
async fn an_exclusion_rule_hides_an_opportunity_and_can_be_undone() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "excl",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    ingest_one(
        &db.pool,
        &source,
        "1",
        "Memory Systems Engineer",
        "DRAM and CXL work.",
        Some("DE"),
    )
    .await;
    let profile = seeded_profile(&db.pool, "architecture", &["DRAM"], &["DE"]).await;

    let before = matching::matches_for_profile(
        &db.pool,
        profile.id,
        &MatchQuery {
            q: None,
            limit: 25,
            include_excluded: false,
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(before.eligible, 1);

    let rule = profiles::create_rule(
        &db.pool,
        profiles::NewRule {
            kind: "company".into(),
            value: "Example Silicon GmbH".into(),
            profile_id: None,
            opportunity_id: None,
            note: Some("not hiring in my area".into()),
        },
    )
    .await
    .unwrap();

    let after = matching::matches_for_profile(
        &db.pool,
        profile.id,
        &MatchQuery {
            q: None,
            limit: 25,
            include_excluded: false,
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        after.eligible, 0,
        "a normalized alias of the employer must exclude it"
    );
    assert_eq!(after.excluded, 1);
    assert!(after.items.is_empty());

    // The owner can see what was excluded and why, which is what makes the rule reversible in practice.
    let shown = matching::matches_for_profile(
        &db.pool,
        profile.id,
        &MatchQuery {
            q: None,
            limit: 25,
            include_excluded: true,
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(shown.items.len(), 1);
    assert!(!shown.items[0].result.eligible);
    assert!(shown.items[0].result.exclusions[0]
        .reason
        .contains("excluded the employer"));
    assert!(shown.items[0].result.exclusions[0].reversible);

    // The most relevant exclusion sorts first, so a rule's effect is inspectable rather than buried.
    assert!(
        !shown.items[0].result.factors.is_empty(),
        "the factors it would have scored on are still shown"
    );

    profiles::delete_rule(&db.pool, rule.id).await.unwrap();
    let restored = matching::matches_for_profile(
        &db.pool,
        profile.id,
        &MatchQuery {
            q: None,
            limit: 25,
            include_excluded: false,
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        restored.eligible, 1,
        "deleting the rule restores the opportunity"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn hiding_one_opportunity_leaves_every_other_one_visible() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "hide",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    ingest_one(
        &db.pool,
        &source,
        "1",
        "Memory Systems Engineer",
        "DRAM work.",
        Some("DE"),
    )
    .await;
    ingest_one(
        &db.pool,
        &source,
        "2",
        "Memory Controller Engineer",
        "DRAM work.",
        Some("DE"),
    )
    .await;
    let profile = seeded_profile(&db.pool, "architecture", &["DRAM"], &["DE"]).await;

    let page = matching::matches_for_profile(
        &db.pool,
        profile.id,
        &MatchQuery {
            q: None,
            limit: 25,
            include_excluded: false,
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(page.eligible, 2);
    let target = page.items[0].opportunity.id;

    profiles::create_rule(
        &db.pool,
        profiles::NewRule {
            kind: "hidden".into(),
            value: String::new(),
            profile_id: None,
            opportunity_id: Some(target),
            note: Some("already applied".into()),
        },
    )
    .await
    .unwrap();

    let after = matching::matches_for_profile(
        &db.pool,
        profile.id,
        &MatchQuery {
            q: None,
            limit: 25,
            include_excluded: false,
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(after.eligible, 1, "hiding one must not hide the other");
    assert_ne!(after.items[0].opportunity.id, target);
    db.cleanup().await;
}

#[tokio::test]
async fn feedback_persists_and_records_whether_the_item_was_surfaced() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "fb",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    ingest_one(
        &db.pool,
        &source,
        "1",
        "Memory Systems Engineer",
        "DRAM work.",
        Some("DE"),
    )
    .await;
    ingest_one(
        &db.pool,
        &source,
        "2",
        "Office Manager",
        "Admin work.",
        Some("DE"),
    )
    .await;
    let profile = seeded_profile(&db.pool, "architecture", &["DRAM"], &["DE"]).await;
    let page = matching::matches_for_profile(
        &db.pool,
        profile.id,
        &MatchQuery {
            q: None,
            limit: 1,
            include_excluded: false,
        },
        None,
    )
    .await
    .unwrap();
    let surfaced = page.items[0].opportunity.id;

    digest::record_feedback(
        &db.pool,
        digest::NewFeedback {
            opportunity_id: surfaced,
            profile_id: Some(profile.id),
            verdict: "interested".into(),
            sampled_from: Some("surfaced".into()),
            note: None,
        },
    )
    .await
    .unwrap();

    // Judging only what the ranking already showed would measure it against itself, so the unsurfaced remainder is sampled too.
    let unseen = digest::sample_not_surfaced(&db.pool, &[surfaced], 5)
        .await
        .unwrap();
    assert_eq!(unseen.len(), 1);
    digest::record_feedback(
        &db.pool,
        digest::NewFeedback {
            opportunity_id: unseen[0].0,
            profile_id: Some(profile.id),
            verdict: "not_interested".into(),
            sampled_from: Some("not_surfaced".into()),
            note: None,
        },
    )
    .await
    .unwrap();

    let counts: Vec<(String, i64)> = sqlx::query_as(
        "select sampled_from, count(*) from opportunity_feedback group by sampled_from order by 1",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        counts,
        vec![("not_surfaced".to_string(), 1), ("surfaced".to_string(), 1)]
    );

    // Recording a second verdict for the same pair updates rather than duplicating.
    digest::record_feedback(
        &db.pool,
        digest::NewFeedback {
            opportunity_id: surfaced,
            profile_id: Some(profile.id),
            verdict: "not_interested".into(),
            sampled_from: Some("surfaced".into()),
            note: Some("changed my mind".into()),
        },
    )
    .await
    .unwrap();
    let total: i64 = sqlx::query_scalar("select count(*) from opportunity_feedback")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(total, 2);
    db.cleanup().await;
}

#[tokio::test]
async fn a_digest_is_generated_stored_and_carries_real_links_and_deadlines() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "dg",
        "Example Institut",
        SourceEndpoint::AcademicHtml {
            url: "https://example-institut.test/abschlussarbeiten".into(),
            pattern: None,
            selectors: None,
        },
    )
    .await;
    let mut thesis = academic_posting(
        "t1",
        "Masterarbeit: Seitenmigration",
        AdvertisedState::OpenTopic,
    );
    thesis.deadline_at = Some(Utc::now() + chrono::Duration::days(10));
    ingest(
        &db.pool,
        &source,
        scan_of(RunOutcome::OkComplete, vec![thesis]),
    )
    .await
    .unwrap();

    let mut profile = Profile::blank("research", "Research");
    profile.terms = vec!["Masterarbeit".into()];
    let profile = profiles::create(&db.pool, profile).await.unwrap();

    let generated = digest::generate(
        &db.pool,
        profile.id,
        digest::DigestFormat::Markdown,
        25,
        None,
    )
    .await
    .unwrap();
    assert_eq!(generated.item_count, 1);
    assert_eq!(
        generated.deadline_count, 1,
        "a deadline inside the horizon must be called out"
    );
    assert!(
        generated
            .body
            .contains("https://example-institut.test/arbeiten/t1"),
        "the real link must be present"
    );
    assert!(generated.body.contains("Deadlines within"));
    assert!(
        generated.body.contains("may not be a funded vacancy"),
        "an open topic stays flagged in the digest"
    );
    assert!(
        generated.body.contains("lexical ranking only"),
        "the ranking method must be stated"
    );

    let csv = digest::generate(&db.pool, profile.id, digest::DigestFormat::Csv, 25, None)
        .await
        .unwrap();
    assert!(csv
        .body
        .lines()
        .next()
        .unwrap()
        .starts_with("score,model,title"));
    assert!(csv
        .body
        .contains("https://example-institut.test/arbeiten/t1"));

    // Digests persist so the owner can look back at what was surfaced.
    let stored = digest::list(&db.pool, 10).await.unwrap();
    assert_eq!(stored.len(), 2);
    db.cleanup().await;
}

#[tokio::test]
async fn an_opportunity_with_no_location_still_reaches_the_ranking() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "sparse",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    ingest_one(
        &db.pool,
        &source,
        "1",
        "Memory Systems Engineer",
        "DRAM and CXL work.",
        None,
    )
    .await;
    let profile = seeded_profile(&db.pool, "architecture", &["DRAM"], &["DE"]).await;

    let page = matching::matches_for_profile(
        &db.pool,
        profile.id,
        &MatchQuery {
            q: None,
            limit: 25,
            include_excluded: false,
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        page.eligible, 1,
        "a parsing gap must not drop a real opening"
    );
    assert!(page.items[0]
        .result
        .uncertainties
        .iter()
        .any(|u| u.field == "location"));

    // Unless the owner explicitly asks to hide them, in which case the reason is stated.
    let mut strict = profiles::get(&db.pool, profile.id).await.unwrap().profile;
    strict.include_unknown_location = false;
    profiles::update(&db.pool, profile.id, strict)
        .await
        .unwrap();
    let hidden = matching::matches_for_profile(
        &db.pool,
        profile.id,
        &MatchQuery {
            q: None,
            limit: 25,
            include_excluded: true,
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(hidden.eligible, 0);
    assert!(hidden.items[0].result.exclusions[0]
        .reason
        .contains("location unknown"));
    db.cleanup().await;
}

#[tokio::test]
async fn matching_works_with_no_embedding_provider_and_never_claims_a_model_ran() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "nollm",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    ingest_one(
        &db.pool,
        &source,
        "1",
        "Memory Systems Engineer",
        "DRAM work.",
        Some("DE"),
    )
    .await;
    let profile = seeded_profile(&db.pool, "architecture", &["DRAM"], &["DE"]).await;

    let page = matching::matches_for_profile(
        &db.pool,
        profile.id,
        &MatchQuery {
            q: None,
            limit: 25,
            include_excluded: false,
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        page.semantic,
        opportunity_core::embeddings::SemanticStatus::Disabled
    );
    assert!(page.ranking.contains("no embedding provider is configured"));
    assert_eq!(
        page.items[0].result.model,
        opportunity_domain::LEXICAL_MODEL
    );
    assert!(!page.items[0]
        .result
        .factors
        .iter()
        .any(|f| f.name == "semantic similarity"));
    assert!(!page.ranking.to_lowercase().contains(" ai "));
    db.cleanup().await;
}

#[tokio::test]
async fn academic_and_employment_opportunities_are_classified_and_both_remain_findable() {
    let db = require_db!();
    let company = register(
        &db.pool,
        "co",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    let institute = register(
        &db.pool,
        "inst",
        "Example Institut",
        SourceEndpoint::AcademicHtml {
            url: "https://example-institut.test/abschlussarbeiten".into(),
            pattern: None,
            selectors: None,
        },
    )
    .await;
    ingest_one(
        &db.pool,
        &company,
        "1",
        "Memory Systems Engineer",
        "DRAM work.",
        Some("DE"),
    )
    .await;
    ingest(
        &db.pool,
        &institute,
        scan_of(
            RunOutcome::OkComplete,
            vec![academic_posting(
                "t1",
                "Masterarbeit: Seitenmigration",
                AdvertisedState::OpenTopic,
            )],
        ),
    )
    .await
    .unwrap();

    // A profile that wants both kinds sees both, with each labelled for what it is.
    let mut profile = Profile::blank("both", "Both");
    profile.terms = vec!["DRAM".into(), "Masterarbeit".into()];
    profile.job_kinds = vec!["employment".into(), "master_thesis".into()];
    let profile = profiles::create(&db.pool, profile).await.unwrap();
    let page = matching::matches_for_profile(
        &db.pool,
        profile.id,
        &MatchQuery {
            q: None,
            limit: 25,
            include_excluded: false,
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(page.eligible, 2);
    let states: Vec<&str> = page
        .items
        .iter()
        .map(|i| i.opportunity.advertised_state.as_str())
        .collect();
    assert!(states.contains(&"advertised") && states.contains(&"open_topic"));

    // A thesis-only profile excludes the employment role with a reversible reason rather than dropping it silently.
    let mut thesis_only = profiles::get(&db.pool, profile.id).await.unwrap().profile;
    thesis_only.job_kinds = vec!["master_thesis".into()];
    profiles::update(&db.pool, profile.id, thesis_only)
        .await
        .unwrap();
    let narrowed = matching::matches_for_profile(
        &db.pool,
        profile.id,
        &MatchQuery {
            q: None,
            limit: 25,
            include_excluded: true,
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(narrowed.eligible, 1);
    let refused = narrowed.items.iter().find(|i| !i.result.eligible).unwrap();
    assert!(refused.result.exclusions[0]
        .reason
        .contains("does not look for employment"));
    db.cleanup().await;
}

#[tokio::test]
async fn an_incomplete_academic_sweep_never_closes_what_it_did_not_see() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "paged",
        "Example Institut",
        SourceEndpoint::AcademicHtml {
            url: "https://example-institut.test/abschlussarbeiten".into(),
            pattern: Some("successfactors".into()),
            selectors: None,
        },
    )
    .await;
    let full = vec![
        academic_posting("a", "Masterarbeit A", AdvertisedState::OpenTopic),
        academic_posting("b", "Masterarbeit B", AdvertisedState::OpenTopic),
        academic_posting("c", "Masterarbeit C", AdvertisedState::OpenTopic),
    ];
    ingest(&db.pool, &source, scan_of(RunOutcome::OkComplete, full))
        .await
        .unwrap();
    assert_eq!(open_postings(&db.pool, source.id).await, 3);

    // The listing paginated and only the first page was read, which the adapter reports as partial.
    let mut truncated = scan_of(
        RunOutcome::Partial,
        vec![academic_posting(
            "a",
            "Masterarbeit A",
            AdvertisedState::OpenTopic,
        )],
    );
    truncated.diagnostic =
        Some("listing states 3 results but the sweep read 1 after 1 pages".into());
    let summary = ingest(&db.pool, &source, truncated).await.unwrap();

    assert!(!summary.complete);
    assert_eq!(summary.postings_marked_missing, 0);
    assert_eq!(summary.postings_closed, 0);
    assert_eq!(
        open_postings(&db.pool, source.id).await,
        3,
        "a truncated listing must not close the two it did not reach"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn only_one_refresh_can_run_at_a_time_even_across_processes() {
    let db = require_db!();
    // The scheduler loop and a manual refresh are different code paths that could be different processes. Two sweeps of one source at once would interleave their disappearance decisions, which is exactly what the completeness rules cannot survive.
    let first = scheduler::WorkLease::try_acquire(&db.pool)
        .await
        .unwrap()
        .expect("the first refresh takes the lock");

    // A second pool onto the same database is a separate connection, which is how a second process would look. An advisory lock is per database, so both must point at the same one.
    let other = opportunity_core::connect(&db.url).await.unwrap();
    assert!(
        scheduler::WorkLease::try_acquire(&other)
            .await
            .unwrap()
            .is_none(),
        "a second refresh must not start while one is running"
    );

    first.release().await;
    let after = scheduler::WorkLease::try_acquire(&other)
        .await
        .unwrap()
        .expect("releasing lets the next refresh start");
    after.release().await;
    other.close().await;
    db.cleanup().await;
}

#[tokio::test]
async fn a_catalogue_larger_than_the_shortlist_says_so_rather_than_scoring_a_silent_subset() {
    let db = require_db!();
    let source = register(
        &db.pool,
        "shortlist",
        "Example Silicon",
        SourceEndpoint::Greenhouse {
            board_token: "example".into(),
        },
    )
    .await;
    // One sweep carrying all four. Distinct titles, because a numeric suffix normalizes away and they would dedupe into one; one sweep, because four sweeps of one posting each would close the earliest as missing.
    let postings = [
        "Memory Systems Engineer",
        "DRAM Architect",
        "CXL Platform Engineer",
        "Cache Hierarchy Researcher",
    ]
    .iter()
    .enumerate()
    .map(|(n, title)| {
        let mut p = posting(&n.to_string(), title, Some("Munich, Germany"));
        p.description_html = Some("<p>DRAM and CXL work.</p>".into());
        p.country_code = Some("DE".into());
        p.city = Some("Munich".into());
        p
    })
    .collect();
    ingest(&db.pool, &source, scan_of(RunOutcome::OkComplete, postings))
        .await
        .unwrap();
    let profile = seeded_profile(&db.pool, "architecture", &["DRAM"], &["DE"]).await;
    let query = MatchQuery {
        q: None,
        limit: 25,
        include_excluded: false,
    };

    // Serialized through one test because the bound is process-wide.
    std::env::set_var("MATCH_SHORTLIST", "2");
    let capped = matching::matches_for_profile(&db.pool, profile.id, &query, None)
        .await
        .unwrap();
    assert_eq!(capped.total_considered, 2);
    assert!(
        capped.ranking.contains("most recently seen"),
        "a truncated ranking must say so: {}",
        capped.ranking
    );

    std::env::set_var("MATCH_SHORTLIST", "50");
    let whole = matching::matches_for_profile(&db.pool, profile.id, &query, None)
        .await
        .unwrap();
    std::env::remove_var("MATCH_SHORTLIST");
    assert_eq!(whole.total_considered, 4);
    assert!(
        !whole.ranking.contains("most recently seen"),
        "nothing was left out, so nothing should claim it was: {}",
        whole.ranking
    );
    db.cleanup().await;
}

#[tokio::test]
async fn a_browser_run_and_a_sweep_cannot_have_the_machine_at_the_same_time() {
    let db = require_db!();
    // One lock covers both kinds of heavy work, because on a small machine they do not fit together:
    // a sweep running underneath a submission is enough for the kernel to kill the browser mid-form.
    let sweep = scheduler::WorkLease::try_acquire(&db.pool)
        .await
        .unwrap()
        .expect("the sweep takes the lock");

    let browser = opportunity_core::connect(&db.url).await.unwrap();
    let waited = std::time::Instant::now();
    let refused = scheduler::WorkLease::acquire_within(&browser, Duration::from_millis(600))
        .await
        .unwrap();
    assert!(
        refused.is_none(),
        "the browser must not start while a sweep is running"
    );
    assert!(
        waited.elapsed() >= Duration::from_millis(600),
        "waiting has to actually wait, {:?}",
        waited.elapsed()
    );
    assert!(
        waited.elapsed() < Duration::from_secs(5),
        "and it has to give up: {:?}",
        waited.elapsed()
    );

    // When the sweep finishes, the waiting browser run gets in promptly rather than on the next tick.
    sweep.release().await;
    let took = std::time::Instant::now();
    let slot = scheduler::WorkLease::acquire_within(&browser, Duration::from_secs(5))
        .await
        .unwrap()
        .expect("the machine is free now");
    assert!(
        took.elapsed() < Duration::from_secs(2),
        "{:?}",
        took.elapsed()
    );
    slot.release().await;
    browser.close().await;
    db.cleanup().await;
}

#[tokio::test]
async fn a_lease_whose_holder_vanishes_does_not_block_the_next_one() {
    let db = require_db!();
    // A process killed mid-run cannot release anything. Its connection dies with it and PostgreSQL
    // drops the session's locks, which is the whole reason this is an advisory lock on a detached
    // connection rather than a flag in a table or in memory.
    let abandoned = opportunity_core::connect(&db.url).await.unwrap();
    let lease = scheduler::WorkLease::try_acquire(&abandoned)
        .await
        .unwrap()
        .expect("the lock is free to start with");
    drop(lease);
    abandoned.close().await;

    let after = scheduler::WorkLease::acquire_within(&db.pool, Duration::from_secs(5))
        .await
        .unwrap();
    assert!(
        after.is_some(),
        "a lease nobody released must not lock the machine out forever"
    );
    after.unwrap().release().await;
    db.cleanup().await;
}

#[tokio::test]
async fn holding_the_lease_leaves_the_connection_pool_usable() {
    let db = require_db!();
    // The lease detaches its connection from the pool instead of borrowing one, so a small pool is
    // not spent on waiting. With four connections and a lease held, four queries still run.
    let lease = scheduler::WorkLease::try_acquire(&db.pool)
        .await
        .unwrap()
        .expect("the lock is free");

    for _ in 0..4 {
        let one: i32 = sqlx::query_scalar("select 1")
            .fetch_one(&db.pool)
            .await
            .expect("the pool still serves queries while heavy work runs");
        assert_eq!(one, 1);
    }
    // And the work itself can still read and write, which is what it spends most of its time doing.
    let sources = sources::list(&db.pool).await.unwrap();
    assert!(sources.is_empty() || !sources.is_empty());

    lease.release().await;
    db.cleanup().await;
}
