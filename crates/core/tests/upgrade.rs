//! Upgrading a database that already holds data, from the migration level production runs (0005) to
//! the latest, over real PostgreSQL with synthetic rows. A fresh-database test cannot see a migration
//! that rejects or rewrites existing rows; this one can. It also covers the legacy notification
//! cutover (0011): what existed before email delivery is kept and never emailed, and what the
//! upgraded application queues still is.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use opportunity_core::{alerts, applications, mail, ops, preferences, product, Db};
use serde_json::{json, Value};
use sqlx::{Connection, Executor, PgConnection};
use uuid::Uuid;

/// The last migration production has applied (`1043ab9`, read from the VM on 2026-10-27).
const PRODUCTION_LEVEL: &str = "0005";

const MIGRATIONS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../migrations");
const CUTOVER: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../migrations/0011_legacy_notification_cutover.sql"
);
const SUPPRESSED: &str = "Created before email delivery existed; kept in the app and not emailed.";

/// Every table that exists at the production level and that a later migration alters or reads.
const TABLES: [&str; 16] = [
    "owners",
    "organizations",
    "sources",
    "opportunities",
    "postings",
    "profiles",
    "exclusion_rules",
    "documents",
    "applications",
    "application_attempts",
    "application_events",
    "review_tasks",
    "notifications",
    "applicant_facts",
    "authorization_policies",
    "users",
];

/// Synthetic rows at the 0005 schema: one application in every status 0005 allows, an attempt for
/// each that reached the employer, and every notification kind 0005 allows, a month old: the one for
/// 'submitted' marked seen in the app, the other three never seen.
const SEED: &str = r#"
insert into organizations (normalized_name, display_name) values ('example silicon', 'Example Silicon');
insert into sources (key, kind, display_name, endpoint, enabled)
    values ('example-lever', 'lever', 'Example', '{"kind":"lever","site":"example"}', true);
insert into opportunities (organization_id, dedupe_key, canonical_title, kind, advertised_state, deadline_at)
    select (select id from organizations), 'opp-' || s, 'Firmware Engineer ' || s, 'employment', 'advertised', now() + interval '60 hours'
    from unnest(array['saved','preparing','needs_review','ready','submitting','submitted','submission_unknown',
                      'interview','rejected','withdrawn','failed']) s;
insert into postings (source_id, native_id, opportunity_id, url, title, content_hash)
    select (select id from sources), o.dedupe_key, o.id, 'https://jobs.lever.co/example/' || o.dedupe_key, o.canonical_title, 'h-' || o.dedupe_key
    from opportunities o;
insert into profiles (owner_id, key, display_name) values ((select id from owners), 'firmware', 'Firmware and platform systems');
insert into exclusion_rules (owner_id, kind, value, normalized_value) values ((select id from owners), 'company', 'Blocked Corp', 'blocked corp');
insert into documents (owner_id, kind, label, file_path, content_hash, byte_size, media_type)
    values ((select id from owners), 'cv', 'old-cv.pdf', 'private/documents/old-cv.pdf', 'cv-hash', 100, 'application/pdf');
insert into applications (owner_id, opportunity_id, profile_id, status, cv_document_id)
    select (select id from owners), o.id, (select id from profiles), substr(o.dedupe_key, 5), (select id from documents)
    from opportunities o;
insert into application_attempts (application_id, idempotency_key, destination, payload_hash, state)
    select a.id, 'attempt-' || a.status, 'https://jobs.lever.co/example', 'payload-hash',
           case a.status when 'submitted' then 'submitted' when 'submission_unknown' then 'unknown'
                         when 'failed' then 'failed' else 'started' end
    from applications a where a.status in ('submitting', 'submitted', 'submission_unknown', 'failed');
insert into application_events (application_id, kind) select id, 'created' from applications;
insert into review_tasks (application_id, field_name, question_text)
    select id, 'visa', 'Do you need a visa?' from applications where status = 'needs_review';
insert into notifications (owner_id, application_id, kind, dedupe_key, created_at, delivered_at)
    select (select id from owners), a.id, k, k || ':' || a.id, now() - interval '30 days',
           case when k = 'submitted' then now() - interval '29 days' end
    from applications a, unnest(array['action_required', 'submitted', 'failed', 'submission_uncertain']) k
    where a.status = 'submitted';
insert into applicant_facts (owner_id, key, value, category) values ((select id from owners), 'email', 'owner@example.test', 'contact');
insert into authorization_policies (owner_id, name) values ((select id from owners), 'Lever, firmware');
insert into users (owner_id, email, display_name, role) values ((select id from owners), 'owner@example.test', 'Owner', 'owner');
insert into user_sessions (user_id, token_hash, expires_at) select id, '\x01'::bytea, now() + interval '1 day' from users;
"#;

async fn rows(pool: &Db, table: &str) -> Vec<Value> {
    let rows: Value = sqlx::query_scalar(&format!(
        "select coalesce(jsonb_agg(to_jsonb(t) order by t.id), '[]') from {table} t"
    ))
    .fetch_one(pool)
    .await
    .unwrap();
    rows.as_array().unwrap().clone()
}

struct Scratch {
    pool: Db,
    admin: PgConnection,
    name: String,
}

impl Scratch {
    async fn create() -> Option<Self> {
        let base = std::env::var("DATABASE_URL").ok()?;
        let (prefix, _) = base.rsplit_once('/').unwrap();
        let name = format!("opportunity_test_{}", Uuid::new_v4().simple());
        let mut admin = PgConnection::connect(&format!("{prefix}/postgres"))
            .await
            .unwrap();
        admin
            .execute(format!(r#"create database "{name}""#).as_str())
            .await
            .unwrap();
        let pool = opportunity_core::connect(&format!("{prefix}/{name}"))
            .await
            .unwrap();
        Some(Self { pool, admin, name })
    }

    async fn cleanup(mut self) {
        self.pool.close().await;
        let _ = self
            .admin
            .execute(format!(r#"drop database if exists "{}" with (force)"#, self.name).as_str())
            .await;
    }
}

macro_rules! require_db {
    () => {
        match Scratch::create().await {
            Some(db) => db,
            None => {
                eprintln!("SKIPPED: DATABASE_URL is not set, so this PostgreSQL integration test did not run");
                return;
            }
        }
    };
}

/// Apply only the migrations up to `level`, through the same runner and checksums as the full set.
async fn migrate_to(pool: &Db, level: &str) {
    let dir = std::env::temp_dir().join(format!(
        "opportunity-migrations-{}",
        Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&dir).unwrap();
    for entry in std::fs::read_dir(MIGRATIONS).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        if name.ends_with(".sql") && name[..4] <= *level {
            std::fs::copy(&path, dir.join(&name)).unwrap();
        }
    }
    let migrator = sqlx::migrate::Migrator::new(Path::new(&dir)).await.unwrap();
    let included = migrator.iter().count();
    let result = migrator.run(pool).await;
    std::fs::remove_dir_all(&dir).unwrap();
    result.unwrap();
    assert_eq!(included, level.parse::<usize>().unwrap());
}

/// Each notification as "kind state", plus "seen" once the owner has marked it so in the app.
async fn notification_states(pool: &Db) -> Vec<String> {
    sqlx::query_scalar(
        "select kind || ' ' || delivery_state || case when delivered_at is null then '' else ' seen' end
         from notifications order by kind",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

/// A loopback stand-in for the mail provider: it accepts every send and reports every message
/// delivered. It logs "POST <idempotency key>" per send and "GET <path>" per status check, so a test
/// sees exactly what would have been emailed. Nothing leaves the machine.
fn fake_mail_provider() -> (mail::Transport, Arc<Mutex<Vec<String>>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let transport = mail::Transport {
        base: format!("http://{}", listener.local_addr().unwrap()),
        key: "synthetic".into(),
        from: "test@example.test".into(),
        to: "owner@example.test".into(),
    };
    let log = Arc::new(Mutex::new(Vec::new()));
    let requests = log.clone();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut parts = line.split_whitespace();
            let (method, path) = (parts.next().unwrap(), parts.next().unwrap());
            let (mut key, mut length) = (String::new(), 0);
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                let Some((name, value)) = header.split_once(':') else {
                    break;
                };
                match name.to_ascii_lowercase().as_str() {
                    "idempotency-key" => key = value.trim().to_owned(),
                    "content-length" => length = value.trim().parse().unwrap(),
                    _ => {}
                }
            }
            reader.read_exact(&mut vec![0; length]).unwrap();
            let reply = if method == "POST" {
                requests.lock().unwrap().push(format!("POST {key}"));
                json!({ "id": format!("re_{key}") })
            } else {
                requests.lock().unwrap().push(format!("GET {path}"));
                json!({ "last_event": "delivered" })
            }
            .to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                reply.len()
            )
            .unwrap();
        }
    });
    (transport, log)
}

#[tokio::test]
async fn upgrading_from_the_production_level_keeps_every_row_and_value() {
    let db = require_db!();
    let pool = db.pool.clone();

    migrate_to(&pool, PRODUCTION_LEVEL).await;
    sqlx::raw_sql(SEED).execute(&pool).await.unwrap();
    let mut before = Vec::new();
    for table in TABLES {
        before.push(rows(&pool, table).await);
    }

    opportunity_core::migrate(&pool)
        .await
        .expect("the upgrade accepts every row the production level allows");

    // Every row is still there, and every value it had is unchanged; migrations only add columns.
    for (table, old_rows) in TABLES.iter().zip(&before) {
        let new_rows = rows(&pool, table).await;
        assert_eq!(new_rows.len(), old_rows.len(), "{table}: row count changed");
        for (old, new) in old_rows.iter().zip(&new_rows) {
            for (column, value) in old.as_object().unwrap() {
                assert_eq!(
                    &new[column], value,
                    "{table}.{column} changed for {}",
                    old["id"]
                );
            }
        }
    }

    // Columns added after 0005 start at their documented defaults on existing rows.
    let count = |sql: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(
        count("select count(*) from applications where auto_resume").await,
        0,
        "nothing old is queued for automation"
    );
    assert_eq!(
        count("select count(*) from review_tasks where options <> '[]'").await,
        0
    );
    assert_eq!(
        count("select count(*) from documents where candidate is not null").await,
        0
    );
    assert_eq!(
        count("select count(*) from authorization_policies where profile_id is not null").await,
        0
    );
    assert_eq!(
        count("select count(*) from notifications where delivered_at is not null").await,
        1,
        "a delivered notification stays delivered"
    );
    assert_eq!(
        count("select count(*) from sources s where not exists (select 1 from seed_ledger l where l.key = s.key)").await,
        0,
        "existing sources count as seeded, so a deleted one is never recreated"
    );
    assert_eq!(
        count("select count(*) from user_sessions").await,
        1,
        "signed-in sessions survive"
    );
    assert_eq!(count("select count(*) from opportunity_marks").await, 0);
    assert_eq!(
        count("select count(*) from notification_preferences").await,
        0
    );
    let applied: Vec<(i64, bool)> =
        sqlx::query_as("select version, success from _sqlx_migrations order by version")
            .fetch_all(&pool)
            .await
            .unwrap();
    let files = std::fs::read_dir(MIGRATIONS)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|x| x == "sql")
        })
        .count();
    assert_eq!(applied.len(), files);
    assert!(applied.iter().all(|(_, success)| *success));

    // Every month-old notification is kept. The one seen in the app stays seen. The three never seen
    // are suppressed, so turning mail on does not email a backlog. Only delivery_state and
    // delivery_error, both added by 0006, differ from 0006's defaults; every 0005 column is unchanged
    // (checked above).
    assert_eq!(
        notification_states(&pool).await,
        [
            "action_required suppressed",
            "failed suppressed",
            "submission_uncertain suppressed",
            "submitted pending seen"
        ]
    );
    let reasons: Vec<Option<String>> =
        sqlx::query_scalar("select delivery_error from notifications order by kind")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        reasons,
        [Some(SUPPRESSED), Some(SUPPRESSED), Some(SUPPRESSED), None].map(|r| r.map(String::from))
    );
    let status = ops::status(&pool).await.unwrap();
    assert_eq!(
        status["notifications"]["pending"], 0,
        "nothing old waits to be emailed"
    );
    assert_eq!(
        status["notifications"]["oldest_pending_minutes"],
        Value::Null
    );
    assert_eq!(status["notifications"]["undeliverable"], 0);

    // What the upgraded application queues is emailed; nothing from before the upgrade is.
    let submitted: Uuid =
        sqlx::query_scalar("select id from applications where status = 'submitted'")
            .fetch_one(&pool)
            .await
            .unwrap();
    applications::notify(
        &pool,
        Some(submitted),
        "manual_required",
        "after-upgrade",
        json!({}),
    )
    .await
    .unwrap();
    let new: Uuid =
        sqlx::query_scalar("select id from notifications where dedupe_key = 'after-upgrade'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let (transport, requests) = fake_mail_provider();
    mail::deliver(&pool, &transport).await.unwrap();
    assert_eq!(*requests.lock().unwrap(), [format!("POST {new}")]);
    assert_eq!(
        notification_states(&pool).await,
        [
            "action_required suppressed",
            "failed suppressed",
            "manual_required accepted",
            "submission_uncertain suppressed",
            "submitted pending seen"
        ]
    );

    // The product works on the upgraded data, and its counts read the old rows correctly.
    let home = product::dashboard(&pool).await.unwrap();
    assert_eq!(
        home["mail"],
        json!([{ "application_id": submitted, "state": "seen", "error": null }]),
        "a notification seen before the upgrade never reads as queued email"
    );
    assert_eq!(home["applications"].as_array().unwrap().len(), 11);
    let counts = &home["application_counts"];
    assert_eq!(
        [
            &counts["confirmed"],
            &counts["reported"],
            &counts["uncertain"],
            &counts["failed"],
            &counts["needs_review"],
            &counts["queued"]
        ],
        [1, 2, 1, 1, 1, 0].map(Value::from).each_ref()
    );
    assert_eq!(
        preferences::notification_preferences(&pool)
            .await
            .unwrap()
            .reminder_days,
        [7, 3, 1]
    );
    let saved: Uuid =
        sqlx::query_scalar("select opportunity_id from applications where status = 'saved'")
            .fetch_one(&pool)
            .await
            .unwrap();
    preferences::set_mark(&pool, saved, "saved", true)
        .await
        .unwrap();
    // Reminders go to the five applications still in progress, never to ones applied for or closed.
    assert_eq!(alerts::queue_deadline_reminders(&pool).await.unwrap(), 5);

    opportunity_core::migrate(&pool)
        .await
        .expect("a second run on the upgraded database is a no-op");

    db.cleanup().await;
}

/// A database that ran 0006 to 0010 before the cutover existed, such as a local or review copy.
/// Since 0006 its application queued a new notification, and its outbox already sent one legacy row
/// that is still waiting for the provider's confirmation. The cutover must leave both alone.
#[tokio::test]
async fn a_database_already_past_0010_keeps_mail_queued_since_and_the_cutover_runs_once() {
    let db = require_db!();
    let pool = db.pool.clone();
    migrate_to(&pool, PRODUCTION_LEVEL).await;
    sqlx::raw_sql(SEED).execute(&pool).await.unwrap();
    migrate_to(&pool, "0010").await;
    let submitted: Uuid =
        sqlx::query_scalar("select id from applications where status = 'submitted'")
            .fetch_one(&pool)
            .await
            .unwrap();
    applications::notify(
        &pool,
        Some(submitted),
        "manual_required",
        "since-0006",
        json!({}),
    )
    .await
    .unwrap();
    sqlx::query(
        "update notifications set delivery_state = 'accepted', delivery_receipt = 're_earlier',
                first_attempt_at = now() - interval '2 hours', last_attempt_at = now() - interval '2 hours'
         where kind = 'failed'",
    )
    .execute(&pool)
    .await
    .unwrap();

    opportunity_core::migrate(&pool).await.unwrap();
    assert_eq!(
        notification_states(&pool).await,
        [
            "action_required suppressed",
            "failed accepted",
            "manual_required pending",
            "submission_uncertain suppressed",
            "submitted pending seen"
        ]
    );

    // Running the cutover again, by hand or from a restored copy, changes nothing.
    let before = rows(&pool, "notifications").await;
    sqlx::raw_sql(&std::fs::read_to_string(CUTOVER).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(rows(&pool, "notifications").await, before);
    opportunity_core::migrate(&pool).await.unwrap();
    assert_eq!(rows(&pool, "notifications").await, before);

    // The row queued since 0006 is sent. The one already sent is only confirmed, never sent again.
    // The seen and the suppressed ones are never contacted.
    let since: Uuid =
        sqlx::query_scalar("select id from notifications where dedupe_key = 'since-0006'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let (transport, requests) = fake_mail_provider();
    mail::deliver(&pool, &transport).await.unwrap();
    assert_eq!(
        *requests.lock().unwrap(),
        ["GET /emails/re_earlier".to_owned(), format!("POST {since}")]
    );
    assert_eq!(
        notification_states(&pool).await,
        [
            "action_required suppressed",
            "failed delivered seen",
            "manual_required accepted",
            "submission_uncertain suppressed",
            "submitted pending seen"
        ]
    );

    db.cleanup().await;
}
