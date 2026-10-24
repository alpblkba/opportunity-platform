//! What an unauthenticated caller can and cannot do, over real HTTP.
//!
//! These drive the assembled router on a real socket with a real database, because the thing being
//! tested is the arrangement of middleware, routes and cookies, not the helpers underneath. A test
//! that called `authenticate()` directly would pass happily while every private route was open.

use std::net::SocketAddr;

use opportunity_api::{router, AccessMode, AppState};
use opportunity_core::auth::{self, Role};
use opportunity_core::Db;
use sqlx::{Connection, Executor, PgConnection};
use uuid::Uuid;

const PASSWORD: &str = "correct horse battery staple";

struct TestServer {
    dist: std::path::PathBuf,
    base: String,
    client: reqwest::Client,
    pool: Db,
    admin_url: String,
    name: String,
}

impl TestServer {
    async fn start(mode: AccessMode) -> Option<Self> {
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

        // A stand-in for the built frontend. Whether this is served publicly is a per-mode
        // decision, and without a bundle on disk the question cannot be asked at all.
        let dist =
            std::env::temp_dir().join(format!("opportunity-dist-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(dist.join("assets")).expect("dist directory");
        std::fs::write(
            dist.join("index.html"),
            "<!doctype html><title>Opportunity platform</title>",
        )
        .expect("index");
        std::fs::write(dist.join("assets/app.js"), "// the bundle").expect("asset");

        let state = AppState {
            db: pool.clone(),
            http: opportunity_adapters::HttpClient::new().expect("http client"),
            embeddings: None,
            web_dist: Some(dist.clone()),
            access: mode,
            // The test server speaks plain HTTP, so the cookie cannot be marked Secure or the
            // browser-equivalent client would never send it back. Production sets this true; the
            // attributes themselves are asserted below.
            secure_cookies: false,
            logins: std::sync::Arc::new(opportunity_api::auth::LoginLimiter::default()),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router(state)).await.ok();
        });
        Some(Self {
            dist,
            base: format!("http://127.0.0.1:{}", addr.port()),
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            pool,
            admin_url,
            name,
        })
    }

    async fn owner(&self) -> auth::Account {
        auth::create_account(
            &self.pool,
            "owner@example.test",
            "Owner",
            Role::Owner,
            Some(PASSWORD),
        )
        .await
        .expect("create the owner account")
    }

    /// A request with explicit control over the session cookie and the Origin, because both are
    /// exactly what is under test.
    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        cookie: Option<&str>,
        origin: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> reqwest::Response {
        let mut request = self
            .client
            .request(method, format!("{}{}", self.base, path))
            .header(
                reqwest::header::HOST,
                self.base.trim_start_matches("http://"),
            );
        if let Some(cookie) = cookie {
            request = request.header(reqwest::header::COOKIE, cookie);
        }
        if let Some(origin) = origin {
            request = request.header(reqwest::header::ORIGIN, origin);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        request.send().await.expect("the server answered")
    }

    fn same_origin(&self) -> String {
        self.base.clone()
    }

    /// A request carrying arbitrary headers, which is how an assertion and a cookie are presented
    /// together without the client inventing either.
    async fn send_with(
        &self,
        method: reqwest::Method,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<serde_json::Value>,
    ) -> reqwest::Response {
        let mut request = self
            .client
            .request(method, format!("{}{}", self.base, path))
            .header(
                reqwest::header::HOST,
                self.base.trim_start_matches("http://"),
            );
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        request.send().await.expect("the server answered")
    }

    /// A POST with headers and a same-origin Origin, for logging in where an assertion is also
    /// required.
    async fn send_with_body(
        &self,
        path: &str,
        headers: &[(&str, &str)],
        body: serde_json::Value,
    ) -> reqwest::Response {
        let origin = self.same_origin();
        let mut all: Vec<(&str, &str)> = headers.to_vec();
        all.push(("origin", origin.as_str()));
        self.send_with(reqwest::Method::POST, path, &all, Some(body))
            .await
    }

    async fn login(&self, email: &str, password: &str) -> reqwest::Response {
        let origin = self.same_origin();
        self.send(
            reqwest::Method::POST,
            "/api/auth/login",
            None,
            Some(&origin),
            Some(serde_json::json!({ "email": email, "password": password })),
        )
        .await
    }

    /// The cookie a successful login handed back, in the form a browser would send it.
    fn cookie_from(response: &reqwest::Response) -> Option<String> {
        let header = response
            .headers()
            .get(reqwest::header::SET_COOKIE)?
            .to_str()
            .ok()?;
        let pair = header.split(';').next()?;
        Some(pair.trim().to_owned())
    }

    async fn cleanup(self) {
        let _ = std::fs::remove_dir_all(&self.dist);
        self.pool.close().await;
        let mut admin = PgConnection::connect(&self.admin_url)
            .await
            .expect("connect");
        admin
            .execute(format!(r#"drop database if exists "{}" with (force)"#, self.name).as_str())
            .await
            .ok();
    }
}

/// Start a server, or fail the test.
///
/// These tests are the security evidence for this service. Skipping them silently when the database
/// is absent made the whole suite pass on a machine where none of it ran, which is the one outcome
/// worse than failing. ALLOW_SKIP_DB_TESTS=1 is the explicit opt-out for working offline; the
/// release command (scripts/verify-security.sh) never sets it.
macro_rules! server {
    ($mode:expr) => {
        match TestServer::start($mode).await {
            Some(server) => server,
            None if std::env::var("ALLOW_SKIP_DB_TESTS").as_deref() == Ok("1") => {
                eprintln!("skipping by ALLOW_SKIP_DB_TESTS=1: no DATABASE_URL");
                return;
            }
            None => panic!(
                "DATABASE_URL is not set, so this security test did not run. \
                 Start PostgreSQL and set DATABASE_URL, or set ALLOW_SKIP_DB_TESTS=1 to work \
                 offline knowing the security suite is not being checked."
            ),
        }
    };
}

const TEAM: &str = "example.cloudflareaccess.com";
const AUDIENCE: &str = "test-application-audience";
const KID: &str = "test-key-1";

/// A Cloudflare application whose keys are already in hand, so a test can present an assertion the
/// server genuinely verifies rather than one it was told to trust.
async fn cloudflare_access() -> std::sync::Arc<opportunity_api::access::CloudflareAccess> {
    let access = std::sync::Arc::new(
        opportunity_api::access::CloudflareAccess::new(
            TEAM.into(),
            AUDIENCE.into(),
            vec!["owner@example.test".into()],
        )
        .expect("configuration is valid"),
    );
    let mut parts = include_str!("access_jwk.txt").split_whitespace();
    access
        .seed_keys_for_test(vec![opportunity_api::access::Jwk {
            kid: KID.to_owned(),
            n: parts.next().expect("modulus").to_owned(),
            e: parts.next().expect("exponent").to_owned(),
        }])
        .await;
    access
}

/// A real RS256 assertion, signed with the repository's throwaway test key.
fn assertion(email: &str, expires_in: chrono::Duration) -> String {
    #[derive(serde::Serialize)]
    struct Claims {
        aud: String,
        iss: String,
        exp: i64,
        iat: i64,
        email: String,
        sub: String,
    }
    let now = chrono::Utc::now();
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some(KID.to_owned());
    jsonwebtoken::encode(
        &header,
        &Claims {
            aud: AUDIENCE.into(),
            iss: format!("https://{TEAM}"),
            exp: (now + expires_in).timestamp(),
            iat: now.timestamp(),
            email: email.to_owned(),
            sub: "test-subject".into(),
        },
        &jsonwebtoken::EncodingKey::from_rsa_pem(include_str!("access-test-key.txt").as_bytes())
            .expect("the test key parses"),
    )
    .expect("signing works")
}

#[tokio::test]
async fn the_private_interface_is_closed_until_a_password_opens_it() {
    let server = server!(AccessMode::Native);
    server.owner().await;

    // Nothing, with no session.
    for path in [
        "/api/stats",
        "/api/applications",
        "/api/facts",
        "/api/profiles",
    ] {
        let response = server
            .send(reqwest::Method::GET, path, None, None, None)
            .await;
        assert_eq!(response.status(), 401, "{path} answered without a session");
    }
    // Health stays open, because something has to be able to probe it.
    let health = server
        .send(reqwest::Method::GET, "/health", None, None, None)
        .await;
    assert_eq!(health.status(), 200);

    // The right password opens it.
    let response = server.login("owner@example.test", PASSWORD).await;
    assert_eq!(response.status(), 200);
    let cookie = TestServer::cookie_from(&response).expect("a session cookie");
    let attributes = response
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(attributes.contains("HttpOnly"), "{attributes}");
    assert!(attributes.contains("SameSite=Lax"), "{attributes}");
    assert!(attributes.contains("Path=/"), "{attributes}");
    assert!(
        !attributes.contains("Domain="),
        "a host-only cookie names no domain: {attributes}"
    );

    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["email"], "owner@example.test");
    assert_eq!(body["role"], "owner");

    let stats = server
        .send(
            reqwest::Method::GET,
            "/api/stats",
            Some(&cookie),
            None,
            None,
        )
        .await;
    assert_eq!(
        stats.status(),
        200,
        "the session should open the private API"
    );
    server.cleanup().await;
}

#[tokio::test]
async fn a_wrong_password_and_an_unknown_address_are_indistinguishable() {
    let server = server!(AccessMode::Native);
    server.owner().await;

    let wrong = server.login("owner@example.test", "not the password").await;
    let unknown = server.login("nobody@example.test", PASSWORD).await;

    assert_eq!(wrong.status(), 401);
    assert_eq!(unknown.status(), 401);
    let wrong_body: serde_json::Value = wrong.json().await.unwrap();
    let unknown_body: serde_json::Value = unknown.json().await.unwrap();
    // The decisive assertion: nothing in the answer says whether the account exists.
    assert_eq!(wrong_body, unknown_body);
    server.cleanup().await;
}

#[tokio::test]
async fn a_burst_of_guesses_is_refused_before_it_becomes_a_search() {
    let server = server!(AccessMode::Native);
    server.owner().await;

    let mut refused = None;
    for attempt in 0..8 {
        let response = server.login("owner@example.test", "wrong").await;
        if response.status() == 429 {
            refused = Some((attempt, response));
            break;
        }
    }
    let (attempt, response) = refused.expect("guessing must be bounded");
    assert!(attempt <= 6, "it took {attempt} attempts to be refused");
    assert!(
        response
            .headers()
            .contains_key(reqwest::header::RETRY_AFTER),
        "a refusal should say when to come back"
    );

    // And the lockout applies to the real password too: an attacker guessing does not get a free
    // oracle by occasionally trying the right one.
    let correct = server.login("owner@example.test", PASSWORD).await;
    assert_eq!(correct.status(), 429);
    server.cleanup().await;
}

#[tokio::test]
async fn a_session_ends_when_it_is_revoked_tampered_with_or_expired() {
    let server = server!(AccessMode::Native);
    let owner = server.owner().await;
    let origin = server.same_origin();

    let response = server.login("owner@example.test", PASSWORD).await;
    let cookie = TestServer::cookie_from(&response).unwrap();
    assert_eq!(
        server
            .send(
                reqwest::Method::GET,
                "/api/auth/session",
                Some(&cookie),
                None,
                None
            )
            .await
            .status(),
        200
    );

    // A cookie with one character changed is not a session.
    let tampered = {
        let (name, value) = cookie.split_once('=').unwrap();
        let mut bytes = value.to_owned();
        bytes.pop();
        bytes.push(if value.ends_with('a') { 'b' } else { 'a' });
        format!("{name}={bytes}")
    };
    assert_eq!(
        server
            .send(
                reqwest::Method::GET,
                "/api/auth/session",
                Some(&tampered),
                None,
                None
            )
            .await
            .status(),
        401
    );
    // Nor is a cookie that is not a token at all.
    assert_eq!(
        server
            .send(
                reqwest::Method::GET,
                "/api/auth/session",
                Some("opportunity_session=../../etc/passwd"),
                None,
                None
            )
            .await
            .status(),
        401
    );

    // Logging out ends it, and says so by clearing the cookie.
    let logout = server
        .send(
            reqwest::Method::POST,
            "/api/auth/logout",
            Some(&cookie),
            Some(&origin),
            None,
        )
        .await;
    assert_eq!(logout.status(), 200);
    let cleared = logout
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
    assert_eq!(
        server
            .send(
                reqwest::Method::GET,
                "/api/stats",
                Some(&cookie),
                None,
                None
            )
            .await
            .status(),
        401,
        "a revoked session must not keep working"
    );

    // An expired session is refused by the lookup, not by anything remembering to check.
    let fresh = server.login("owner@example.test", PASSWORD).await;
    let fresh_cookie = TestServer::cookie_from(&fresh).unwrap();
    sqlx::query(
        "update user_sessions set expires_at = now() - interval '1 minute' where user_id = $1",
    )
    .bind(owner.id)
    .execute(&server.pool)
    .await
    .unwrap();
    assert_eq!(
        server
            .send(
                reqwest::Method::GET,
                "/api/stats",
                Some(&fresh_cookie),
                None,
                None
            )
            .await
            .status(),
        401,
        "an expired session must not keep working"
    );
    server.cleanup().await;
}

#[tokio::test]
async fn a_request_from_another_site_cannot_act_with_the_session() {
    let server = server!(AccessMode::Native);
    server.owner().await;
    let response = server.login("owner@example.test", PASSWORD).await;
    let cookie = TestServer::cookie_from(&response).unwrap();

    // The cookie is valid; the request is not, because it came from somewhere else.
    for path in ["/api/auth/logout", "/api/refresh", "/api/profiles"] {
        let forged = server
            .send(
                reqwest::Method::POST,
                path,
                Some(&cookie),
                Some("https://evil.example"),
                Some(serde_json::json!({})),
            )
            .await;
        assert_eq!(forged.status(), 403, "{path} accepted a cross-site request");
    }

    // Login itself is protected the same way: a form posted from another site is login CSRF.
    let forged_login = server
        .send(
            reqwest::Method::POST,
            "/api/auth/login",
            None,
            Some("https://evil.example"),
            Some(serde_json::json!({ "email": "owner@example.test", "password": PASSWORD })),
        )
        .await;
    assert_eq!(forged_login.status(), 403);

    // A reading request from elsewhere still cannot read anything, because it is the session that
    // is checked and the browser would not attach one on a cross-site fetch anyway.
    let read = server
        .send(
            reqwest::Method::GET,
            "/api/stats",
            None,
            Some("https://evil.example"),
            None,
        )
        .await;
    assert_eq!(read.status(), 401);
    server.cleanup().await;
}

/// The release matrix, as one test per mode so a failure names the mode it happened in.
///
/// Each mode is checked with every combination of credentials that could be presented: neither,
/// one, the other, both. The point is not that the right one works; it is that nothing else does.
#[tokio::test]
async fn cloudflare_mode_takes_an_assertion_and_nothing_else() {
    let server = server!(AccessMode::Cloudflare(cloudflare_access().await));
    server.owner().await;
    let valid = assertion("owner@example.test", chrono::Duration::hours(1));

    // 1. A valid assertion and no native cookie: the workspace works. This is the rollback target,
    //    and the deployment that runs today.
    let with_assertion = server
        .send_with(
            reqwest::Method::GET,
            "/api/stats",
            &[("cf-access-jwt-assertion", valid.as_str())],
            None,
        )
        .await;
    assert_eq!(
        with_assertion.status(),
        200,
        "a Cloudflare identity must be enough here"
    );

    // The interface asks this before it renders anything, so it has to agree with the gate.
    let session = server
        .send_with(
            reqwest::Method::GET,
            "/api/auth/session",
            &[("cf-access-jwt-assertion", valid.as_str())],
            None,
        )
        .await;
    assert_eq!(
        session.status(),
        200,
        "the interface must not be sent to a password form"
    );
    let body: serde_json::Value = session.json().await.unwrap();
    assert_eq!(body["email"], "owner@example.test");
    assert_eq!(body["via"], "cloudflare-access");

    // 2. A native cookie and no assertion: rejected. A session must not bypass Access.
    let login = server.login("owner@example.test", PASSWORD).await;
    let cookie = TestServer::cookie_from(&login).expect("a session cookie");
    assert_eq!(
        server
            .send(
                reqwest::Method::GET,
                "/api/stats",
                Some(&cookie),
                None,
                None
            )
            .await
            .status(),
        401,
        "a native session must not open anything where Cloudflare is the boundary"
    );

    // Neither, and a forged one.
    assert_eq!(
        server
            .send(reqwest::Method::GET, "/api/stats", None, None, None)
            .await
            .status(),
        401
    );
    let forged = assertion("someone-else@example.test", chrono::Duration::hours(1));
    assert_eq!(
        server
            .send_with(
                reqwest::Method::GET,
                "/api/stats",
                &[("cf-access-jwt-assertion", forged.as_str())],
                None
            )
            .await
            .status(),
        401,
        "an identity outside the allowlist is refused even with a real signature"
    );
    // And the interface itself stays behind the gate in this mode, exactly as before.
    assert_eq!(
        server
            .send(reqwest::Method::GET, "/", None, None, None)
            .await
            .status(),
        401
    );
    server.cleanup().await;
}

#[tokio::test]
async fn both_mode_requires_both_and_says_which_is_missing() {
    let server = server!(AccessMode::Both(cloudflare_access().await));
    server.owner().await;
    let valid = assertion("owner@example.test", chrono::Duration::hours(1));

    // 3. Assertion only: rejected, and told that a password is what is missing, so the interface
    //    shows a login form instead of an error.
    let assertion_only = server
        .send_with(
            reqwest::Method::GET,
            "/api/stats",
            &[("cf-access-jwt-assertion", valid.as_str())],
            None,
        )
        .await;
    assert_eq!(assertion_only.status(), 401);
    let body: serde_json::Value = assertion_only.json().await.unwrap();
    assert_eq!(body["needs"], "native");

    // 4. Native cookie only: rejected, because Cloudflare is still in front.
    let login = server
        .send_with_body(
            "/api/auth/login",
            &[("cf-access-jwt-assertion", valid.as_str())],
            serde_json::json!({ "email": "owner@example.test", "password": PASSWORD }),
        )
        .await;
    assert_eq!(login.status(), 200);
    let cookie = TestServer::cookie_from(&login).expect("a session cookie");
    assert_eq!(
        server
            .send(
                reqwest::Method::GET,
                "/api/stats",
                Some(&cookie),
                None,
                None
            )
            .await
            .status(),
        401,
        "the assertion is still required"
    );

    // 5. Both: the workspace works.
    let both = server
        .send_with(
            reqwest::Method::GET,
            "/api/stats",
            &[
                ("cf-access-jwt-assertion", valid.as_str()),
                ("cookie", cookie.as_str()),
            ],
            None,
        )
        .await;
    assert_eq!(both.status(), 200, "both credentials present must work");
    server.cleanup().await;
}

#[tokio::test]
async fn native_mode_takes_a_session_and_nothing_else() {
    let server = server!(AccessMode::Native);
    server.owner().await;

    // 6. No cookie: the private API rejects, and says a password is what is missing.
    let refused = server
        .send(reqwest::Method::GET, "/api/stats", None, None, None)
        .await;
    assert_eq!(refused.status(), 401);
    let body: serde_json::Value = refused.json().await.unwrap();
    assert_eq!(body["needs"], "native");

    // An assertion is not an identity here: Access is no longer in front, so a header anyone could
    // have attached must open nothing.
    let valid = assertion("owner@example.test", chrono::Duration::hours(1));
    assert_eq!(
        server
            .send_with(
                reqwest::Method::GET,
                "/api/stats",
                &[("cf-access-jwt-assertion", valid.as_str())],
                None
            )
            .await
            .status(),
        401,
        "a Cloudflare header must not authenticate anything in native mode"
    );

    // 7. A valid owner cookie: the workspace works.
    let login = server.login("owner@example.test", PASSWORD).await;
    let cookie = TestServer::cookie_from(&login).unwrap();
    assert_eq!(
        server
            .send(
                reqwest::Method::GET,
                "/api/stats",
                Some(&cookie),
                None,
                None
            )
            .await
            .status(),
        200
    );
    server.cleanup().await;
}

/// 8. A non-owner account reaches nothing, at either end: it cannot log in, and a session it
///    somehow held would stop working.
#[tokio::test]
async fn a_non_owner_account_cannot_reach_owner_data() {
    let server = server!(AccessMode::Native);
    let owner = server.owner().await;

    // Creating one is refused, which is the first line of defence.
    for role in [Role::Agent, Role::User] {
        let refused = auth::create_account(
            &server.pool,
            "other@example.test",
            "Other",
            role,
            Some(PASSWORD),
        )
        .await;
        assert!(refused.is_err(), "{role:?} accounts must be refused");
    }

    // Force one into the database anyway, as a migration or a mistake might, and check that the
    // application refuses it rather than trusting that it cannot exist.
    let smuggled: uuid::Uuid = sqlx::query_scalar(
        "insert into users (owner_id, email, display_name, role, password_hash)
         select owner_id, 'agent@example.test', 'Agent', 'agent', password_hash from users where id = $1
         returning id",
    )
    .bind(owner.id)
    .fetch_one(&server.pool)
    .await
    .expect("insert the smuggled account");

    // It has the owner's own password hash, so this fails on the role and nothing else.
    assert_eq!(
        server.login("agent@example.test", PASSWORD).await.status(),
        401,
        "a non-owner account must not be able to log in"
    );

    // And a session minted for it directly opens nothing.
    let issued = auth::begin_session(&server.pool, smuggled)
        .await
        .expect("a session row");
    let cookie = format!("opportunity_session={}", issued.token);
    for path in [
        "/api/stats",
        "/api/facts",
        "/api/applications",
        "/api/auth/session",
    ] {
        assert_eq!(
            server
                .send(reqwest::Method::GET, path, Some(&cookie), None, None)
                .await
                .status(),
            401,
            "{path} was readable by a non-owner session"
        );
    }
    server.cleanup().await;
}

/// 12. The routes that had to become public must publish nothing.
#[tokio::test]
async fn the_public_surface_carries_no_private_data() {
    let server = server!(AccessMode::Native);
    server.owner().await;

    // The login endpoints answer without a session; none of them may leak anything about the owner
    // or the catalogue.
    let session = server
        .send(reqwest::Method::GET, "/api/auth/session", None, None, None)
        .await;
    assert_eq!(session.status(), 401);
    let body = session.text().await.unwrap();
    assert!(
        !body.contains("owner@example.test"),
        "the gate named the account: {body}"
    );

    let health = server
        .send(reqwest::Method::GET, "/health", None, None, None)
        .await;
    let health_body = health.text().await.unwrap();
    assert!(health_body.contains("status"));
    assert!(
        !health_body.contains("@"),
        "health leaked an address: {health_body}"
    );

    // A failed login says nothing about the account either.
    let refused = server.login("owner@example.test", "wrong").await;
    let refused_body = refused.text().await.unwrap();
    assert!(!refused_body.contains("Owner"), "{refused_body}");
    assert!(!refused_body.contains("argon2"), "{refused_body}");
    server.cleanup().await;
}

#[tokio::test]
async fn an_account_that_cannot_log_in_does_not() {
    let server = server!(AccessMode::Native);
    let owner = server.owner().await;

    // An agent has no password, so the password path must refuse it rather than treat an absent
    // hash as a match.
    // An empty password matches nothing, and neither does a password for an address that has none.
    assert_eq!(server.login("owner@example.test", "").await.status(), 401);
    assert_eq!(server.login("nobody@example.test", "").await.status(), 401);

    // A disabled account stops working without its password changing.
    sqlx::query("update users set disabled_at = now() where id = $1")
        .bind(owner.id)
        .execute(&server.pool)
        .await
        .unwrap();
    assert_eq!(
        server.login("owner@example.test", PASSWORD).await.status(),
        401
    );
    server.cleanup().await;
}

#[tokio::test]
async fn a_second_account_cannot_be_created_while_isolation_is_unproven() {
    let server = server!(AccessMode::Native);
    server.owner().await;

    // There is no registration route at all: the only way in is the CLI on the machine.
    let registration = server
        .send(
            reqwest::Method::POST,
            "/api/auth/register",
            None,
            Some(&server.same_origin()),
            Some(serde_json::json!({ "email": "someone@example.test", "password": PASSWORD })),
        )
        .await;
    // 404 from the API nest, or 405 from the static fallback that catches unknown paths. Either
    // way there is no such endpoint; what matters is that it is not a success.
    assert!(
        registration.status().is_client_error(),
        "a registration endpoint answered {}",
        registration.status()
    );
    assert_eq!(
        auth::list_accounts(&server.pool).await.unwrap().len(),
        1,
        "nothing may create a second account"
    );

    // And the CLI path refuses too, because no query filters by owner yet.
    let second_owner = auth::create_account(
        &server.pool,
        "other@example.test",
        "Other",
        Role::Owner,
        Some(PASSWORD),
    )
    .await;
    assert!(second_owner.is_err(), "a second owner must be refused");
    let extra_user = auth::create_account(
        &server.pool,
        "tester@example.test",
        "Tester",
        Role::User,
        Some(PASSWORD),
    )
    .await;
    assert!(extra_user.is_err(), "additional accounts must be refused");
    server.cleanup().await;
}
