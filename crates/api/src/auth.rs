//! Native login: the routes a browser talks to, and the cookie that carries the session.
//!
//! These are the only endpoints under `/api` that an unauthenticated caller may reach, and they are
//! mounted outside the identity middleware deliberately rather than by exception. Everything else
//! stays behind it.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use opportunity_core::auth::{self, Account};
use serde::{Deserialize, Serialize};

use crate::routes::AppState;

/// The cookie the browser carries.
///
/// `__Host-` is a promise enforced by the browser: Secure, path `/`, and no Domain attribute, so the
/// cookie belongs to exactly this hostname and cannot be set by a sibling subdomain. It needs
/// HTTPS, which production has and a loopback development server does not, so the name changes with
/// the deployment rather than the guarantee being quietly dropped in production.
pub const SECURE_COOKIE: &str = "__Host-opportunity_session";
pub const PLAIN_COOKIE: &str = "opportunity_session";

/// How many failures before an address is made to wait, and for how long.
///
/// Per address rather than per source: behind a tunnel every request arrives from the connector, so
/// a client address is not something this process can trust. The account being attacked is.
///
/// The wait doubles with each further failure up to a ceiling, and a success clears it. A ceiling
/// rather than an ever-growing lockout because this is a single-owner service: an attacker who can
/// reach the login page can otherwise keep the owner out of their own account for ever by guessing
/// wrong on a schedule. Fifteen minutes makes guessing useless and keeps that denial bounded.
const MAX_FAILURES: u32 = 5;
const LOCKOUT_BASE: Duration = Duration::from_secs(60);
const LOCKOUT_CEILING: Duration = Duration::from_secs(15 * 60);

/// How many addresses the limiter remembers.
///
/// A hard ceiling, not a cleanup rule: a flood of distinct addresses would otherwise grow the map
/// as fast as it arrives, and "retain the unexpired" retains all of them. At this size the whole
/// structure is a few tens of kilobytes.
const MAX_TRACKED_ADDRESSES: usize = 512;

/// The longest address this service will even hash into a key.
const MAX_EMAIL_BYTES: usize = 254;

/// How many logins may be in flight at once.
///
/// Each one holds an Argon2 permit or waits for one, and each waiting request is a connection and a
/// task. Without a ceiling, a flood would queue on the semaphore and the queue itself would be the
/// resource exhausted. Refusing early is cheaper than queueing politely.
const MAX_LOGINS_IN_FLIGHT: usize = 8;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct SessionBody {
    pub email: String,
    pub display_name: String,
    pub role: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs: Option<&'static str>,
    /// How this request was authenticated, so the interface can say what it is looking at.
    pub via: &'static str,
}

impl SessionBody {
    fn of(account: &Account) -> Self {
        Self {
            email: account.email.clone(),
            display_name: account.display_name.clone(),
            role: account.role.as_str(),
            via: "password",
            needs: None,
        }
    }

    /// The same answer for an identity that came from somewhere other than a password, so the
    /// interface needs no special case for a deployment that has no accounts.
    fn of_identity(identity: &crate::access::Identity) -> Self {
        Self {
            email: identity.email.clone(),
            display_name: identity
                .display_name
                .clone()
                .unwrap_or_else(|| identity.email.clone()),
            role: identity.role.unwrap_or("owner"),
            via: identity.via,
            needs: None,
        }
    }
}

/// Failed attempts, in memory.
///
/// A table would survive a restart, which sounds better than it is: the thing being bounded is a
/// burst, and a restart of this process is not something an attacker can cause. One map, no new
/// dependency, no background job.
pub struct LoginLimiter {
    failures: Mutex<HashMap<String, Attempts>>,
    /// Bounds how many logins exist at once, including the ones waiting to hash.
    in_flight: tokio::sync::Semaphore,
}

#[derive(Clone, Copy)]
struct Attempts {
    count: u32,
    last: Instant,
}

impl Default for LoginLimiter {
    fn default() -> Self {
        Self {
            failures: Mutex::new(HashMap::new()),
            in_flight: tokio::sync::Semaphore::new(MAX_LOGINS_IN_FLIGHT),
        }
    }
}

/// The key an address is tracked under: trimmed, lowercased and cut to a length a real address
/// could have, so a megabyte of "email" costs one bounded key rather than a megabyte of map.
fn limiter_key(email: &str) -> String {
    let trimmed = email.trim().to_lowercase();
    match trimmed.char_indices().nth(MAX_EMAIL_BYTES) {
        Some((cut, _)) => trimmed[..cut].to_owned(),
        None => trimmed,
    }
}

impl LoginLimiter {
    /// A permit for one login, or nothing because too many are already in flight.
    fn admit(&self) -> Option<tokio::sync::SemaphorePermit<'_>> {
        self.in_flight.try_acquire().ok()
    }

    /// How long an address waits after this many failures: nothing until the threshold, then
    /// doubling to a ceiling.
    fn lockout_for(count: u32) -> Duration {
        if count < MAX_FAILURES {
            return Duration::ZERO;
        }
        let steps = (count - MAX_FAILURES).min(8);
        LOCKOUT_BASE
            .saturating_mul(1u32 << steps)
            .min(LOCKOUT_CEILING)
    }

    /// Whether this address is currently refused, and for how long.
    fn locked_for(&self, key: &str) -> Option<Duration> {
        let mut failures = self.failures.lock().ok()?;
        let attempts = failures.get(key).copied()?;
        let wait = Self::lockout_for(attempts.count);
        let since = attempts.last.elapsed();
        if since < wait {
            Some(wait - since)
        } else {
            // Long past any lockout it could earn, so it no longer deserves a slot in a bounded map.
            if since >= LOCKOUT_CEILING {
                failures.remove(key);
            }
            None
        }
    }

    fn record_failure(&self, key: &str) {
        if let Ok(mut failures) = self.failures.lock() {
            if !failures.contains_key(key) && failures.len() >= MAX_TRACKED_ADDRESSES {
                // Full. Expired entries go first; if that frees nothing, the one that has been
                // quiet longest goes, which is the one closest to expiring anyway. An address under
                // active attack is therefore the last thing evicted.
                failures.retain(|_, attempts| attempts.last.elapsed() < LOCKOUT_CEILING);
                if failures.len() >= MAX_TRACKED_ADDRESSES {
                    if let Some(oldest) = failures
                        .iter()
                        .min_by_key(|(_, attempts)| attempts.last)
                        .map(|(key, _)| key.clone())
                    {
                        failures.remove(&oldest);
                    }
                }
            }
            let entry = failures.entry(key.to_owned()).or_insert(Attempts {
                count: 0,
                last: Instant::now(),
            });
            entry.count = entry.count.saturating_add(1);
            entry.last = Instant::now();
        }
    }

    fn clear(&self, key: &str) {
        if let Ok(mut failures) = self.failures.lock() {
            failures.remove(key);
        }
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.failures.lock().map(|f| f.len()).unwrap_or(0)
    }
}

/// The public routes. Mounted outside the identity middleware; nothing else is.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/session", get(session))
}

fn cookie_name(state: &AppState) -> &'static str {
    if state.secure_cookies {
        SECURE_COOKIE
    } else {
        PLAIN_COOKIE
    }
}

/// Read the session token out of a request.
pub fn session_token(headers: &HeaderMap, secure: bool) -> Option<String> {
    let name = if secure { SECURE_COOKIE } else { PLAIN_COOKIE };
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key.trim() == name).then(|| value.trim().to_owned())
    })
}

fn set_cookie(state: &AppState, token: &str, max_age: i64) -> HeaderValue {
    let mut cookie = format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        cookie_name(state),
        token,
        max_age
    );
    if state.secure_cookies {
        cookie.push_str("; Secure");
    }
    HeaderValue::from_str(&cookie).expect("a cookie built from a hex token is a valid header")
}

/// Same shape for every refusal, so a caller learns whether it worked and nothing else.
fn refuse(status: StatusCode, detail: &str) -> Response {
    (
        status,
        Json(serde_json::json!({ "error": "unauthorized", "detail": detail })),
    )
        .into_response()
}

async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> Response {
    if !origin_is_same(&headers) {
        // Login CSRF: a form posted from another site would carry its own Origin.
        return refuse(StatusCode::FORBIDDEN, "this request did not come from here");
    }
    // Taken before anything expensive, and held for the whole attempt. A flood is refused here
    // rather than queueing on the hashing permits behind it.
    let Some(_slot) = state.logins.admit() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::RETRY_AFTER, "5".to_string())],
            Json(serde_json::json!({
                "error": "busy",
                "detail": "too many sign-ins at once. Try again in a moment."
            })),
        )
            .into_response();
    };

    let email = limiter_key(&body.email);
    if let Some(remaining) = state.logins.locked_for(&email) {
        let seconds = remaining.as_secs().max(1);
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, seconds.to_string())],
            Json(serde_json::json!({
                "error": "too_many_requests",
                "detail": "too many failed attempts. Try again later."
            })),
        )
            .into_response();
    }

    let Some(account) = auth::authenticate(&state.db, &email, &body.password).await else {
        state.logins.record_failure(&email);
        // One message for a wrong password, an unknown address and a disabled account alike.
        tracing::warn!("a login attempt failed");
        return refuse(
            StatusCode::UNAUTHORIZED,
            "that email and password do not match an account",
        );
    };

    let issued = match auth::begin_session(&state.db, account.id).await {
        Ok(issued) => issued,
        Err(error) => {
            tracing::error!(%error, "could not start a session");
            return refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "could not start a session",
            );
        }
    };
    state.logins.clear(&email);
    tracing::info!(role = account.role.as_str(), "a session began");

    let max_age = (issued.expires_at - chrono::Utc::now())
        .num_seconds()
        .max(0);
    (
        StatusCode::OK,
        [(
            header::SET_COOKIE,
            set_cookie(&state, &issued.token, max_age),
        )],
        Json(SessionBody::of(&account)),
    )
        .into_response()
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !origin_is_same(&headers) {
        return refuse(StatusCode::FORBIDDEN, "this request did not come from here");
    }
    if let Some(token) = session_token(&headers, state.secure_cookies) {
        let _ = auth::revoke_session(&state.db, &token).await;
    }
    // Clearing the cookie regardless: a logout that finds no session still ends with no session.
    (
        StatusCode::OK,
        [(header::SET_COOKIE, set_cookie(&state, "", 0))],
        Json(serde_json::json!({ "status": "signed out" })),
    )
        .into_response()
}

/// Who the caller is, according to whichever mode is configured.
///
/// This is the question the interface asks before it renders anything, so it has to be the same
/// question the gate answers. Asking only about native cookies was the bug that made a
/// Cloudflare-authenticated owner meet a password form in a deployment that has no passwords.
async fn session(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match crate::access::resolve_identity(&state, &headers).await {
        Ok(identity) => (StatusCode::OK, Json(SessionBody::of_identity(&identity))).into_response(),
        Err(error) => error.into_response(),
    }
}

/// Whether an `Origin` header, if there is one, names this same host.
///
/// Comparing against the request's own `Host` rather than a configured value: the browser sets both,
/// an attacker's page cannot change either, and it needs no deployment-specific setting to be right.
pub fn origin_is_same(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        // Absent on same-origin GETs and on anything that is not a browser. A request with no Origin
        // and no cookie can do nothing on anyone's behalf; one with a cookie is handled below.
        return true;
    };
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    origin
        .split_once("://")
        .map(|(_, rest)| rest == host)
        .unwrap_or(false)
}

/// Refuse a cookie-authenticated state change that came from another site.
///
/// `SameSite=Lax` already stops a cross-site POST carrying this cookie, in every browser that
/// honours it. This is the second lock: it costs two header reads and does not depend on the
/// browser having got the first one right.
pub async fn require_same_origin(request: Request, next: Next) -> Response {
    let unsafe_method = !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    if unsafe_method && !origin_is_same(request.headers()) {
        return refuse(StatusCode::FORBIDDEN, "this request did not come from here");
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    #[test]
    fn an_origin_from_somewhere_else_is_refused() {
        let same = headers(&[
            ("origin", "https://opportunities.example.test"),
            ("host", "opportunities.example.test"),
        ]);
        assert!(origin_is_same(&same));

        let elsewhere = headers(&[
            ("origin", "https://evil.example"),
            ("host", "opportunities.example.test"),
        ]);
        assert!(!origin_is_same(&elsewhere));

        // A lookalike that merely ends with the right name is still somewhere else.
        let lookalike = headers(&[
            ("origin", "https://opportunities.example.test.evil.example"),
            ("host", "opportunities.example.test"),
        ]);
        assert!(!origin_is_same(&lookalike));

        // No Origin at all: not a browser form post, and nothing to ride on.
        assert!(origin_is_same(&headers(&[(
            "host",
            "opportunities.example.test"
        )])));
    }

    #[test]
    fn the_session_cookie_is_read_by_the_name_the_deployment_uses() {
        let secure = headers(&[(
            "cookie",
            "other=1; __Host-opportunity_session=abc123; third=2",
        )]);
        assert_eq!(session_token(&secure, true).as_deref(), Some("abc123"));
        // The plain name is not accepted where the secure one is expected, and the reverse.
        assert_eq!(session_token(&secure, false), None);

        let plain = headers(&[("cookie", "opportunity_session=xyz")]);
        assert_eq!(session_token(&plain, false).as_deref(), Some("xyz"));
        assert_eq!(session_token(&plain, true), None);
    }

    #[test]
    fn an_address_is_refused_after_enough_failures_and_released_by_a_success() {
        let limiter = LoginLimiter::default();
        let who = "owner@example.test";
        for _ in 0..MAX_FAILURES {
            assert!(limiter.locked_for(who).is_none());
            limiter.record_failure(who);
        }
        assert!(limiter.locked_for(who).is_some(), "the burst is bounded");

        limiter.clear(who);
        assert!(limiter.locked_for(who).is_none(), "a success starts over");
    }

    #[test]
    fn a_flood_of_distinct_addresses_cannot_grow_the_map_without_limit() {
        let limiter = LoginLimiter::default();
        for n in 0..(MAX_TRACKED_ADDRESSES * 4) {
            limiter.record_failure(&limiter_key(&format!("flood-{n}@example.test")));
        }
        assert!(
            limiter.tracked() <= MAX_TRACKED_ADDRESSES,
            "tracking {} addresses, which is not a bound",
            limiter.tracked()
        );
    }

    #[test]
    fn an_oversized_address_becomes_a_bounded_key() {
        let huge = format!("{}@example.test", "a".repeat(100_000));
        let key = limiter_key(&huge);
        assert!(
            key.len() <= MAX_EMAIL_BYTES + 4,
            "key was {} bytes",
            key.len()
        );
        // And the same address twice is the same key, so a flood of one address is one entry.
        let limiter = LoginLimiter::default();
        limiter.record_failure(&limiter_key(&huge));
        limiter.record_failure(&limiter_key(&huge));
        assert_eq!(limiter.tracked(), 1);
        // Case and surrounding space do not make a new entry either.
        limiter.record_failure(&limiter_key("  OWNER@example.test "));
        limiter.record_failure(&limiter_key("owner@example.test"));
        assert_eq!(limiter.tracked(), 2);
    }

    #[test]
    fn keeping_the_owner_out_for_ever_costs_more_than_it_is_worth() {
        // Repeated failures lengthen the wait, but only to a ceiling: an attacker who can reach the
        // form can lock the owner out for a quarter of an hour at a time, and no longer.
        assert_eq!(LoginLimiter::lockout_for(MAX_FAILURES - 1), Duration::ZERO);
        assert_eq!(LoginLimiter::lockout_for(MAX_FAILURES), LOCKOUT_BASE);
        assert_eq!(
            LoginLimiter::lockout_for(MAX_FAILURES + 1),
            LOCKOUT_BASE * 2
        );
        assert_eq!(
            LoginLimiter::lockout_for(MAX_FAILURES + 50),
            LOCKOUT_CEILING
        );
    }

    #[tokio::test]
    async fn only_so_many_logins_may_be_in_flight_at_once() {
        let limiter = LoginLimiter::default();
        let held: Vec<_> = (0..MAX_LOGINS_IN_FLIGHT)
            .map(|_| limiter.admit().expect("under the ceiling"))
            .collect();
        assert!(
            limiter.admit().is_none(),
            "a flood must be refused rather than queued behind the hashing permits"
        );
        drop(held);
        assert!(limiter.admit().is_some(), "and released when they finish");
    }
}
