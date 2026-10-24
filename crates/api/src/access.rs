//! Access control.
//!
//! This service holds a person's CV, their answers to application questions and the authority to submit applications on their behalf. It must never be reachable by anyone else, and the check that enforces that lives here, in the backend, because frontend routing is decoration rather than security.
//!
//! Two modes. In production the origin sits behind Cloudflare Access, which signs an assertion for every request; this module validates that signature against the team's public keys and checks the identity against an allowlist of exactly the people permitted. In local development there is no Access in front, so the process refuses to listen on anything but loopback. That refusal is the thing that stops an unprotected origin existing by accident.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// How the service decides who is calling.
#[derive(Debug, Clone)]
pub enum AccessMode {
    /// No identity layer in front. Only ever valid on loopback, which `guard_bind_address` enforces.
    LoopbackOnly,
    /// Cloudflare Access terminates identity and signs an assertion this process verifies.
    Cloudflare(Arc<CloudflareAccess>),
    /// Cloudflare Access *and* a native session. The rung of the ladder that lets native login be
    /// exercised in production without opening anything: Cloudflare still decides who reaches the
    /// origin, and the application still decides who is logged in.
    Both(Arc<CloudflareAccess>),
    /// A native session is the boundary. Only correct once the Cloudflare Access policy has been
    /// narrowed to match, which is a deliberate cutover described in docs/AUTHENTICATION.md.
    Native,
}

impl AccessMode {
    /// Whether a native session can authenticate a request in this mode.
    pub fn uses_sessions(&self) -> bool {
        matches!(self, Self::Both(_) | Self::Native)
    }

    /// What the process says about itself at startup.
    pub fn describe(&self) -> String {
        match self {
            Self::LoopbackOnly => {
                "loopback only, no identity layer. Reachable from this machine only.".into()
            }
            Self::Cloudflare(cf) => format!(
                "Cloudflare Access, team {}, {} allowed identity(ies)",
                cf.team_domain,
                cf.allowed_emails.len()
            ),
            Self::Both(cf) => format!(
                "Cloudflare Access (team {}) and a native session, both required",
                cf.team_domain
            ),
            Self::Native => {
                "native sessions. Cloudflare Access must no longer be in front of /login.".into()
            }
        }
    }
}

#[derive(Debug)]
pub struct CloudflareAccess {
    /// The team domain, for example `example.cloudflareaccess.com`. Public keys are fetched from here.
    pub team_domain: String,
    /// The application audience tag. Checking it is what stops an assertion minted for a different application being replayed at this one.
    pub audience: String,
    /// Exactly the identities permitted. Empty is refused at startup rather than meaning "everyone".
    pub allowed_emails: Vec<String>,
    keys: RwLock<Option<CachedKeys>>,
    client: reqwest::Client,
}

#[derive(Debug, Clone)]
struct CachedKeys {
    keys: Vec<Jwk>,
    fetched_at: Instant,
}

/// Public keys are rotated by Cloudflare, so they are refetched periodically rather than pinned.
const KEY_CACHE_TTL: Duration = Duration::from_secs(60 * 30);

/// One public key as Cloudflare publishes it.
#[derive(Debug, Clone, Deserialize)]
pub struct Jwk {
    pub kid: String,
    pub n: String,
    pub e: String,
}

#[derive(Debug, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(Debug, Deserialize)]
struct AccessClaims {
    email: Option<String>,
    sub: Option<String>,
}

/// Who the current request belongs to. Handlers that touch personal data take this, so a route cannot forget to be protected and still compile against them.
#[derive(Debug, Clone, Serialize)]
pub struct Identity {
    pub email: String,
    pub subject: Option<String>,
    pub via: &'static str,
    /// What to call them. Present when an account was involved; otherwise the interface falls back
    /// to the address, which is all an assertion carries.
    pub display_name: Option<String>,
    /// The account, when a native session authenticated this request.
    pub user_id: Option<uuid::Uuid>,
    /// What the data belongs to. Present whenever an account was involved; the ownership work that
    /// would make this filter every query is not done yet, and docs/AUTHENTICATION.md says so.
    pub owner_id: Option<uuid::Uuid>,
    pub role: Option<&'static str>,
}

impl Identity {
    fn from_account(account: &opportunity_core::auth::Account) -> Self {
        Self {
            email: account.email.clone(),
            display_name: Some(account.display_name.clone()),
            subject: Some(account.id.to_string()),
            via: "password",
            user_id: Some(account.id),
            owner_id: Some(account.owner_id),
            role: Some(account.role.as_str()),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AccessError {
    #[error("no access assertion was presented")]
    Missing,
    #[error("the access assertion could not be verified: {0}")]
    Invalid(String),
    #[error("{0} is not permitted to use this service")]
    NotAllowed(String),
    /// Whoever is calling has got as far as the application and has no session. The one refusal the
    /// interface acts on differently, because it is the one a password can fix.
    #[error("this request needs a native session")]
    NeedsNativeLogin,
    #[error("could not fetch the access public keys: {0}")]
    Keys(String),
}

impl IntoResponse for AccessError {
    fn into_response(self) -> Response {
        // Deliberately uniform and uninformative to the caller: an unauthenticated probe learns nothing about which part failed, or whether an identity exists.
        let status = match self {
            Self::Keys(_) => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::UNAUTHORIZED,
        };
        // Which credential is missing is not a secret - the interface has to know whether to show a
        // password form or to say the identity layer in front expired - but nothing here says
        // whether an account exists or which half of a login was wrong.
        let needs = match self {
            Self::NeedsNativeLogin => "native",
            _ => "identity",
        };
        tracing::warn!(reason = %self, "refused a request");
        (
            status,
            Json(serde_json::json!({
                "error": "unauthorized",
                "needs": needs,
                "detail": "this service is private and requires an authorized identity"
            })),
        )
            .into_response()
    }
}

impl CloudflareAccess {
    pub fn new(
        team_domain: String,
        audience: String,
        allowed_emails: Vec<String>,
    ) -> Result<Self, String> {
        if team_domain.trim().is_empty() || audience.trim().is_empty() {
            return Err("cloudflare access needs both a team domain and an audience tag".into());
        }
        if allowed_emails.is_empty() {
            // An empty allowlist must never mean "anyone Cloudflare lets through".
            return Err("cloudflare access needs at least one allowed identity; an empty allowlist is refused".into());
        }
        Ok(Self {
            team_domain: team_domain.trim().trim_end_matches('/').to_owned(),
            audience: audience.trim().to_owned(),
            allowed_emails: allowed_emails
                .into_iter()
                .map(|e| e.trim().to_lowercase())
                .collect(),
            keys: RwLock::new(None),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(|e| e.to_string())?,
        })
    }

    fn certs_url(&self) -> String {
        format!("https://{}/cdn-cgi/access/certs", self.team_domain)
    }

    async fn keys(&self) -> Result<Vec<Jwk>, AccessError> {
        if let Some(cached) = self.keys.read().await.as_ref() {
            if cached.fetched_at.elapsed() < KEY_CACHE_TTL {
                return Ok(cached.keys.clone());
            }
        }
        let response = self
            .client
            .get(self.certs_url())
            .send()
            .await
            .map_err(|e| AccessError::Keys(e.to_string()))?;
        if !response.status().is_success() {
            return Err(AccessError::Keys(format!(
                "certificate endpoint responded {}",
                response.status()
            )));
        }
        let jwks: Jwks = response
            .json()
            .await
            .map_err(|e| AccessError::Keys(e.to_string()))?;
        *self.keys.write().await = Some(CachedKeys {
            keys: jwks.keys.clone(),
            fetched_at: Instant::now(),
        });
        Ok(jwks.keys)
    }

    /// Verify one assertion and return who it belongs to.
    pub async fn verify(&self, token: &str) -> Result<Identity, AccessError> {
        let keys = self.keys().await?;
        self.verify_with_keys(token, &keys)
    }

    /// The whole decision, given the keys. Separated from fetching them so every rule here can be exercised against controlled signed assertions rather than only against malformed ones.
    pub fn verify_with_keys(&self, token: &str, keys: &[Jwk]) -> Result<Identity, AccessError> {
        let header = decode_header(token).map_err(|e| AccessError::Invalid(e.to_string()))?;
        let kid = header
            .kid
            .ok_or_else(|| AccessError::Invalid("assertion names no key".into()))?;
        let jwk = keys
            .iter()
            .find(|k| k.kid == kid)
            .ok_or_else(|| AccessError::Invalid("assertion was signed by an unknown key".into()))?;

        let key = DecodingKey::from_rsa_components(&jwk.n, &jwk.e)
            .map_err(|e| AccessError::Invalid(format!("unusable public key: {e}")))?;
        let mut validation = Validation::new(Algorithm::RS256);
        // Both of these matter: the audience stops an assertion for another application being replayed here, and the issuer stops one from another Cloudflare team.
        validation.set_audience(std::slice::from_ref(&self.audience));
        validation.set_issuer(&[format!("https://{}", self.team_domain)]);
        validation.validate_exp = true;

        let data = decode::<AccessClaims>(token, &key, &validation)
            .map_err(|e| AccessError::Invalid(e.to_string()))?;
        let email = data
            .claims
            .email
            .map(|e| e.trim().to_lowercase())
            .ok_or_else(|| AccessError::Invalid("assertion carries no identity".into()))?;
        if !self.allowed_emails.contains(&email) {
            return Err(AccessError::NotAllowed(email));
        }
        Ok(Identity {
            email,
            display_name: None,
            subject: data.claims.sub,
            via: "cloudflare-access",
            user_id: None,
            owner_id: None,
            role: None,
        })
    }
}

impl CloudflareAccess {
    /// Put keys in the cache so `verify` answers without reaching the network.
    ///
    /// For tests that drive the assembled router in Cloudflare modes: without it they would have to
    /// reach a real team domain, and a test that needs the internet is a test that gets deleted.
    /// Nothing in the binary calls this; the cache is otherwise filled only by a real fetch.
    #[doc(hidden)]
    pub async fn seed_keys_for_test(&self, keys: Vec<Jwk>) {
        *self.keys.write().await = Some(CachedKeys {
            keys,
            fetched_at: Instant::now(),
        });
    }

    /// Exposed for the assertion test suite, which signs controlled assertions and checks which ones are accepted.
    pub fn verify_with_keys_for_test(
        &self,
        token: &str,
        keys: &[Jwk],
    ) -> Result<Identity, AccessError> {
        self.verify_with_keys(token, keys)
    }
}

/// Pull the assertion out of a request. Cloudflare sends it as a header, and as a cookie for browser navigation.
pub fn extract_assertion(request: &Request) -> Option<String> {
    assertion_from(request.headers())
}

pub fn assertion_from(headers: &header::HeaderMap) -> Option<String> {
    if let Some(value) = headers.get("cf-access-jwt-assertion") {
        if let Ok(token) = value.to_str() {
            if !token.trim().is_empty() {
                return Some(token.trim().to_owned());
            }
        }
    }
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookies.split(';').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name.trim() == "CF_Authorization").then(|| value.trim().to_owned())
    })
}

/// Who this request belongs to, in whatever way the configured mode decides.
///
/// One function, used by the middleware and by `/api/auth/session`, so the interface is told exactly
/// what the gate would do rather than asking a different question and guessing from the answer.
/// Each mode is a complete decision: nothing falls through to a weaker check when a stronger one is
/// absent, and no identity is ever built from a header this service has not verified.
pub async fn resolve_identity(
    state: &crate::routes::AppState,
    headers: &header::HeaderMap,
) -> Result<Identity, AccessError> {
    // Cloudflare's half, where the mode has one, verified exactly once per request.
    let cloudflare = match &state.access {
        AccessMode::Cloudflare(access) | AccessMode::Both(access) => {
            let token = assertion_from(headers).ok_or(AccessError::Missing)?;
            Some(access.verify(&token).await?)
        }
        AccessMode::LoopbackOnly | AccessMode::Native => None,
    };

    match &state.access {
        AccessMode::LoopbackOnly => Ok(Identity {
            email: "local-owner".into(),
            display_name: None,
            subject: None,
            via: "loopback",
            user_id: None,
            owner_id: None,
            role: None,
        }),
        // Cloudflare alone is the boundary: no account is required and a session cookie is not an
        // identity. This is what production runs today and what a rollback returns to.
        AccessMode::Cloudflare(_) => Ok(cloudflare.expect("verified above in this mode")),
        // Both halves have to pass. The native half is reported as its own refusal so the interface
        // can show a password form rather than guessing which credential was missing.
        AccessMode::Both(_) => {
            let _ = cloudflare.expect("verified above in this mode");
            native_identity(state, headers)
                .await
                .ok_or(AccessError::NeedsNativeLogin)
        }
        AccessMode::Native => native_identity(state, headers)
            .await
            .ok_or(AccessError::NeedsNativeLogin),
    }
}

async fn native_identity(
    state: &crate::routes::AppState,
    headers: &header::HeaderMap,
) -> Option<Identity> {
    let token = crate::auth::session_token(headers, state.secure_cookies)?;
    let account = opportunity_core::auth::session_account(&state.db, &token).await?;
    Some(Identity::from_account(&account))
}

/// The middleware every protected route runs through.
pub async fn require_identity(
    State(state): State<crate::routes::AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let identity = match resolve_identity(&state, request.headers()).await {
        Ok(identity) => identity,
        Err(error) => return error.into_response(),
    };
    request.extensions_mut().insert(identity);
    next.run(request).await
}

/// Refuse to start in a configuration that would leave personal data reachable without an identity check.
///
/// This is the control that makes "the origin cannot be reached through an unprotected route" true rather than merely intended. Without an identity layer the process will only bind loopback, so reaching it needs access to the machine itself or a tunnel the owner deliberately ran.
pub fn guard_bind_address(mode: &AccessMode, host: &str) -> Result<(), String> {
    if !matches!(mode, AccessMode::LoopbackOnly) {
        return Ok(());
    }
    let loopback = matches!(host, "127.0.0.1" | "::1" | "localhost");
    if loopback {
        Ok(())
    } else {
        Err(format!(
            "refusing to listen on {host} without an identity layer. This service holds personal data and the authority to submit applications. Either bind 127.0.0.1, or configure Cloudflare Access by setting ACCESS_MODE=cloudflare, CF_ACCESS_TEAM_DOMAIN, CF_ACCESS_AUD and ACCESS_ALLOWED_EMAILS."
        ))
    }
}

/// Build the mode from the environment, refusing anything ambiguous.
pub fn mode_from_env() -> Result<AccessMode, String> {
    match std::env::var("ACCESS_MODE").unwrap_or_else(|_| "loopback".into()).as_str() {
        "loopback" => Ok(AccessMode::LoopbackOnly),
        "cloudflare" | "both" => {
            let team_domain = std::env::var("CF_ACCESS_TEAM_DOMAIN")
                .map_err(|_| "ACCESS_MODE=cloudflare needs CF_ACCESS_TEAM_DOMAIN".to_string())?;
            let audience = std::env::var("CF_ACCESS_AUD")
                .map_err(|_| "ACCESS_MODE=cloudflare needs CF_ACCESS_AUD".to_string())?;
            let allowed: Vec<String> = std::env::var("ACCESS_ALLOWED_EMAILS")
                .map_err(|_| "ACCESS_MODE=cloudflare needs ACCESS_ALLOWED_EMAILS".to_string())?
                .split(',')
                .map(|e| e.trim().to_owned())
                .filter(|e| !e.is_empty())
                .collect();
            let access = Arc::new(CloudflareAccess::new(team_domain, audience, allowed)?);
            if std::env::var("ACCESS_MODE").as_deref() == Ok("both") {
                Ok(AccessMode::Both(access))
            } else {
                Ok(AccessMode::Cloudflare(access))
            }
        }
        "native" => Ok(AccessMode::Native),
        other => Err(format!(
            "unknown ACCESS_MODE '{other}'. One of: loopback (local use), cloudflare (today's production), both (Cloudflare and a native session, for testing the cutover), native (native sessions alone)."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_an_identity_layer_the_service_will_only_bind_loopback() {
        let local = AccessMode::LoopbackOnly;
        assert!(guard_bind_address(&local, "127.0.0.1").is_ok());
        assert!(guard_bind_address(&local, "localhost").is_ok());
        assert!(guard_bind_address(&local, "::1").is_ok());

        // The case this exists to prevent: an origin listening on every interface with nothing checking identity.
        let refused = guard_bind_address(&local, "0.0.0.0").unwrap_err();
        assert!(refused.contains("refusing to listen"));
        assert!(refused.contains("ACCESS_MODE=cloudflare"));
        assert!(guard_bind_address(&local, "10.0.0.5").is_err());
    }

    #[test]
    fn with_an_identity_layer_any_bind_address_is_allowed_because_something_is_checking() {
        let access = CloudflareAccess::new(
            "example.cloudflareaccess.com".into(),
            "aud-tag".into(),
            vec!["owner@example.test".into()],
        )
        .unwrap();
        let mode = AccessMode::Cloudflare(Arc::new(access));
        assert!(guard_bind_address(&mode, "0.0.0.0").is_ok());
    }

    #[test]
    fn an_empty_allowlist_is_refused_rather_than_meaning_everyone() {
        let error = CloudflareAccess::new(
            "example.cloudflareaccess.com".into(),
            "aud".into(),
            Vec::new(),
        )
        .unwrap_err();
        assert!(error.contains("empty allowlist is refused"));
    }

    #[test]
    fn a_configuration_missing_its_audience_or_team_is_refused() {
        assert!(
            CloudflareAccess::new(String::new(), "aud".into(), vec!["a@b.test".into()]).is_err()
        );
        assert!(CloudflareAccess::new(
            "team.example".into(),
            String::new(),
            vec!["a@b.test".into()]
        )
        .is_err());
    }

    #[test]
    fn allowed_identities_are_compared_case_insensitively_and_without_surrounding_space() {
        let access = CloudflareAccess::new(
            "example.cloudflareaccess.com".into(),
            "aud".into(),
            vec![" Owner@Example.test ".into()],
        )
        .unwrap();
        assert_eq!(access.allowed_emails, vec!["owner@example.test"]);
    }

    #[test]
    fn the_certificate_url_is_derived_from_the_team_domain() {
        let access = CloudflareAccess::new(
            "example.cloudflareaccess.com/".into(),
            "aud".into(),
            vec!["a@b.test".into()],
        )
        .unwrap();
        assert_eq!(
            access.certs_url(),
            "https://example.cloudflareaccess.com/cdn-cgi/access/certs"
        );
    }

    /// One test, deliberately: `ACCESS_MODE` is process-wide, and two tests setting it at once were
    /// testing each other. That race made a security test fail at random, which is the kind of
    /// failure people learn to rerun rather than read.
    #[test]
    fn the_mode_is_read_strictly_and_never_defaults_to_open() {
        // An unknown value is refused rather than treated as "no checking".
        std::env::set_var("ACCESS_MODE", "disabled");
        assert!(mode_from_env().is_err());

        std::env::set_var("ACCESS_MODE", "loopback");
        assert!(matches!(mode_from_env().unwrap(), AccessMode::LoopbackOnly));

        // A native session is a complete mode and needs no Cloudflare configuration.
        std::env::set_var("ACCESS_MODE", "native");
        assert!(matches!(mode_from_env().unwrap(), AccessMode::Native));

        // The Cloudflare modes refuse to build half-configured.
        for mode in ["cloudflare", "both"] {
            std::env::set_var("ACCESS_MODE", mode);
            std::env::remove_var("CF_ACCESS_TEAM_DOMAIN");
            assert!(
                mode_from_env()
                    .unwrap_err()
                    .contains("CF_ACCESS_TEAM_DOMAIN"),
                "{mode} should name what is missing"
            );
        }
        std::env::remove_var("ACCESS_MODE");
    }
}
