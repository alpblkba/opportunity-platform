//! A standing policy cannot submit, through the direct submit endpoint, an opportunity whose
//! eligibility is not settled; the owner's one-time authorization still can. Over real HTTP with a
//! signed-in owner and a fake worker that inspects honestly and dies on submit before clicking, so a
//! request that passes every gate ends as "failed, nothing sent" and one that is held never begins an
//! attempt. Its own test binary, because the fake worker is selected with process-wide variables.

use std::net::SocketAddr;

use opportunity_api::{router, AccessMode, AppState};
use opportunity_core::applications::{self, NewFact, NewPolicy};
use opportunity_core::auth::{self, Role};
use opportunity_core::{profiles, Db};
use sqlx::{Connection, Executor, PgConnection};
use uuid::Uuid;

const PASSWORD: &str = "correct horse battery staple";

struct TestServer {
    base: String,
    client: reqwest::Client,
    pool: Db,
    admin_url: String,
    name: String,
}

impl TestServer {
    async fn start() -> Option<Self> {
        let base_url = std::env::var("DATABASE_URL").ok()?;
        let (prefix, _) = base_url.rsplit_once('/')?;
        let admin_url = format!("{prefix}/postgres");
        let name = format!("opportunity_auth_{}", Uuid::new_v4().simple());
        let mut admin = PgConnection::connect(&admin_url).await.expect("connect");
        admin
            .execute(format!(r#"create database "{name}""#).as_str())
            .await
            .expect("create test database");
        let pool = opportunity_core::connect(&format!("{prefix}/{name}"))
            .await
            .expect("connect");
        opportunity_core::migrate(&pool).await.expect("migrate");
        let state = AppState {
            db: pool.clone(),
            http: opportunity_adapters::HttpClient::new().expect("http client"),
            embeddings: None,
            web_dist: None,
            access: AccessMode::Native,
            secure_cookies: false,
            logins: std::sync::Arc::new(opportunity_api::auth::LoginLimiter::default()),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router(state)).await.ok();
        });
        Some(Self {
            base: format!("http://127.0.0.1:{}", addr.port()),
            client: reqwest::Client::new(),
            pool,
            admin_url,
            name,
        })
    }

    async fn post(
        &self,
        path: &str,
        cookie: &str,
        body: serde_json::Value,
    ) -> (u16, serde_json::Value) {
        let response = self
            .client
            .post(format!("{}{}", self.base, path))
            .header(reqwest::header::COOKIE, cookie)
            .header(reqwest::header::ORIGIN, &self.base)
            .json(&body)
            .send()
            .await
            .expect("the server answered");
        let status = response.status().as_u16();
        (
            status,
            response.json().await.unwrap_or(serde_json::Value::Null),
        )
    }

    async fn sign_in(&self) -> String {
        auth::create_account(
            &self.pool,
            "owner@example.test",
            "Owner",
            Role::Owner,
            Some(PASSWORD),
        )
        .await
        .unwrap();
        let response = self
            .client
            .post(format!("{}/api/auth/login", self.base))
            .header(reqwest::header::ORIGIN, &self.base)
            .json(&serde_json::json!({ "email": "owner@example.test", "password": PASSWORD }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let header = response
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        header.split(';').next().unwrap().trim().to_owned()
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

/// A synthetic owner: CV, identity facts the fake form asks for, and an opportunity with one open posting.
async fn opportunity(pool: &Db, title: &str, kind: &str, description: &str, url: &str) -> Uuid {
    let org: Uuid = sqlx::query_scalar("insert into organizations (normalized_name, display_name) values ('example silicon', 'Example Silicon') on conflict (normalized_name) do update set updated_at = now() returning id")
        .fetch_one(pool).await.unwrap();
    let source: Uuid = sqlx::query_scalar("insert into sources (key, display_name, kind, endpoint) values ('example-lever', 'Example', 'lever', '{\"kind\":\"lever\",\"site\":\"example\"}') on conflict (key) do update set display_name = excluded.display_name returning id")
        .fetch_one(pool).await.unwrap();
    let id: Uuid = sqlx::query_scalar(
        "insert into opportunities (organization_id, dedupe_key, canonical_title, kind, advertised_state, search_text, country_code, city)
         values ($1, $2, $3, $4, 'advertised', $3 || ' ' || $5, 'DE', 'Munich') returning id")
        .bind(org).bind(Uuid::new_v4().to_string()).bind(title).bind(kind).bind(description).fetch_one(pool).await.unwrap();
    sqlx::query("insert into postings (source_id, native_id, opportunity_id, url, title, description_text, content_hash) values ($1, $2, $3, $4, $5, $6, 'x')")
        .bind(source).bind(id.to_string()).bind(id).bind(url).bind(title).bind(description).execute(pool).await.unwrap();
    id
}

async fn owner_data(pool: &Db, cv_path: &std::path::Path) {
    profiles::seed_default_profiles(pool).await.unwrap();
    std::fs::write(cv_path, "Synthetic Person\nFirmware engineer\n").unwrap();
    let candidate = serde_json::json!({
        "name": null, "email": null, "phone": null, "degree": null, "experience_years": 6,
        "education": ["M.Sc. Computer Science (synthetic)"], "employment": ["Network software engineer (synthetic)"],
        "projects": ["Bootloader for a RISC-V board (synthetic)"], "skills": ["C", "C++", "Linux", "RTOS", "RISC-V"],
    });
    sqlx::query("insert into documents (owner_id, kind, label, file_path, content_hash, byte_size, media_type, candidate, approved)
                 select id, 'cv', 'synthetic.txt', $2, 'synthetic', 1, 'text/plain', $1, true from owners limit 1")
        .bind(candidate).bind(cv_path.to_string_lossy().as_ref()).execute(pool).await.unwrap();
    for (key, value) in [("full name", "A. Example"), ("email", "a@example.test")] {
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

#[tokio::test]
async fn a_standing_policy_cannot_submit_an_unsettled_opportunity_through_the_direct_api() {
    let Some(server) = TestServer::start().await else {
        eprintln!(
            "SKIPPED: DATABASE_URL is not set, so this PostgreSQL integration test did not run"
        );
        return;
    };
    let artefacts =
        std::env::temp_dir().join(format!("opportunity-policy-gate-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&artefacts).unwrap();
    std::env::set_var("BROWSER_WORKER_ENTRY", "../core/tests/fake-worker/cli.js");
    std::env::set_var("BROWSER_WORKER_ARTEFACTS", &artefacts);
    std::env::set_var("FAKE_WORKER_MODE", "die_before_clicking");

    let cookie = server.sign_in().await;
    owner_data(&server.pool, &artefacts.join("synthetic-cv.txt")).await;
    let pool = &server.pool;
    let firmware = profiles::get_by_key(pool, "firmware")
        .await
        .unwrap()
        .unwrap()
        .id;
    let held_url = "http://127.0.0.1:8099/held/apply/single";
    let clear_url = "http://127.0.0.1:8099/clear/apply/single";
    let held = opportunity(pool, "Firmware Engineer", "employment", "Embedded C firmware on an RTOS for RISC-V boards. 3+ years of firmware experience required.", held_url).await;
    let clear = opportunity(
        pool,
        "Firmware Engineering Intern",
        "internship",
        "Embedded C firmware on an RTOS for RISC-V boards; bootloader bring-up.",
        clear_url,
    )
    .await;
    let held_app = applications::create_application(pool, held, Some(firmware))
        .await
        .unwrap()
        .id;
    let clear_app = applications::create_application(pool, clear, Some(firmware))
        .await
        .unwrap()
        .id;
    // The broadest standing policy there is: no profile, the whole source.
    let policy = applications::create_policy(
        pool,
        NewPolicy {
            name: "all of example".into(),
            source_keys: vec!["example-lever".into()],
            max_per_day: Some(5),
            max_total: Some(20),
            expires_in_days: Some(30),
        },
    )
    .await
    .unwrap();
    applications::set_policy_state(pool, policy.id, true, false)
        .await
        .unwrap();
    let attempts = |app: Uuid| {
        sqlx::query_scalar::<_, i64>(
            "select count(*) from application_attempts where application_id = $1",
        )
        .bind(app)
        .fetch_one(pool)
    };

    // Held: refused before any attempt, with the reason, and recorded.
    let (status, body) = server
        .post(
            &format!("/api/applications/{held_app}/submit"),
            &cookie,
            serde_json::json!({ "url": held_url, "source_key": "example-lever" }),
        )
        .await;
    assert!((400..500).contains(&status), "{status} {body}");
    assert!(
        body.to_string().contains("Review before applying")
            && body.to_string().contains("3+ years required"),
        "{body}"
    );
    assert_eq!(attempts(held_app).await.unwrap(), 0);
    let refused: i64 = sqlx::query_scalar("select count(*) from application_events where application_id = $1 and kind = 'submission_refused'").bind(held_app).fetch_one(pool).await.unwrap();
    assert_eq!(refused, 1);

    // Control: a settled opportunity passes the same policy and reaches the worker, which dies before clicking.
    let (status, body) = server
        .post(
            &format!("/api/applications/{clear_app}/submit"),
            &cookie,
            serde_json::json!({ "url": clear_url, "source_key": "example-lever" }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["state"], "failed", "{body}");
    assert_eq!(attempts(clear_app).await.unwrap(), 1);

    // The owner's explicit one-time authorization of this exact payload is the override, and is not held.
    let (status, body) = server
        .post(
            &format!("/api/applications/{held_app}/authorize-submit"),
            &cookie,
            serde_json::json!({ "url": held_url, "source_key": "example-lever" }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = server
        .post(
            &format!("/api/applications/{held_app}/submit"),
            &cookie,
            serde_json::json!({ "url": held_url, "source_key": "example-lever" }),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["state"], "failed", "{body}");
    let kind: String = sqlx::query_scalar(
        "select authorization_kind from application_attempts where application_id = $1",
    )
    .bind(held_app)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(kind, "token");

    std::env::remove_var("BROWSER_WORKER_ENTRY");
    std::env::remove_var("BROWSER_WORKER_ARTEFACTS");
    std::env::remove_var("FAKE_WORKER_MODE");
    let _ = std::fs::remove_dir_all(&artefacts);
    server.cleanup().await;
}
