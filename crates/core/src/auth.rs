//! Accounts, passwords and sessions.
//!
//! The password half is deliberately small: hash with Argon2id, verify in constant work, and never
//! let the answer depend on whether an address exists. The session half is opaque tokens with their
//! hashes in the database, because a token the server can revoke is worth more here than a signed
//! claim it cannot.
//!
//! Nothing in this module logs a password, a token or a hash.

use std::sync::OnceLock;

use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::db::Db;

/// Argon2id cost, sized for a container limited to 256 MiB.
///
/// 19 MiB and two passes is the OWASP floor, and the floor is the right choice here: the limit is
/// the container, not the attacker's patience. Raising memory is the strongest lever available, but
/// this process also holds a shortlist and a connection pool in the same 256 MiB, and a login that
/// triggers the OOM killer protects nobody.
const ARGON_MEMORY_KIB: u32 = 19 * 1024;
const ARGON_PASSES: u32 = 2;
const ARGON_LANES: u32 = 1;

/// How many passwords may be hashed at once.
///
/// Each one allocates ARGON_MEMORY_KIB. Without this, a handful of simultaneous login attempts
/// would be a memory exhaustion attack that needs no credentials to mount.
const MAX_CONCURRENT_HASHES: usize = 2;

/// Short enough that a stolen laptop is not a standing invitation, long enough not to be a nuisance.
const SESSION_TTL_HOURS: i64 = 12;

/// Bounds the work an unauthenticated caller can ask for. Argon2 cost does not depend on length,
/// but reading a megabyte of "password" off the wire does.
const MAX_PASSWORD_BYTES: usize = 1024;
/// Long enough to be worth hashing. This is a personal service with one account, not a public site.
const MIN_PASSWORD_CHARS: usize = 12;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("{0}")]
    Invalid(String),
    #[error("an account already exists for that address")]
    DuplicateEmail,
    #[error("this build permits exactly one owner account; see docs/AUTHENTICATION.md")]
    OwnerExists,
    #[error("no account for that address")]
    NoSuchAccount,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("could not hash the password: {0}")]
    Hashing(String),
}

/// What may this account do. Ownership is a separate question, answered by `owner_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// The person this service exists for.
    Owner,
    /// An additional human account. Refused until isolation is enforced in every query.
    User,
    /// A non-interactive client. No password; authenticates with a session issued out of band.
    Agent,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::User => "user",
            Self::Agent => "agent",
        }
    }

    pub fn parse(value: &str) -> Result<Self, AuthError> {
        match value {
            "owner" => Ok(Self::Owner),
            "user" => Ok(Self::User),
            "agent" => Ok(Self::Agent),
            other => Err(AuthError::Invalid(format!("unknown role '{other}'"))),
        }
    }
}

/// An account, as everything outside this module sees it. There is deliberately no way to get the
/// password hash out of here.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Account {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: Role,
    pub disabled: bool,
}

#[derive(Debug, FromRow)]
struct AccountRow {
    id: Uuid,
    owner_id: Uuid,
    email: String,
    display_name: String,
    role: String,
    password_hash: Option<String>,
    disabled_at: Option<DateTime<Utc>>,
}

impl AccountRow {
    fn account(&self) -> Result<Account, AuthError> {
        Ok(Account {
            id: self.id,
            owner_id: self.owner_id,
            email: self.email.clone(),
            display_name: self.display_name.clone(),
            role: Role::parse(&self.role)?,
            disabled: self.disabled_at.is_some(),
        })
    }
}

const ACCOUNT_COLUMNS: &str = "id, owner_id, email, display_name, role, password_hash, disabled_at";

fn hash_permits() -> &'static Semaphore {
    static PERMITS: OnceLock<Semaphore> = OnceLock::new();
    PERMITS.get_or_init(|| Semaphore::new(MAX_CONCURRENT_HASHES))
}

fn argon() -> Result<Argon2<'static>, AuthError> {
    let params = Params::new(ARGON_MEMORY_KIB, ARGON_PASSES, ARGON_LANES, None)
        .map_err(|e| AuthError::Hashing(e.to_string()))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

fn check_password_rules(password: &str) -> Result<(), AuthError> {
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(AuthError::Invalid(format!(
            "a password longer than {MAX_PASSWORD_BYTES} bytes is refused"
        )));
    }
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(AuthError::Invalid(format!(
            "a password needs at least {MIN_PASSWORD_CHARS} characters"
        )));
    }
    Ok(())
}

/// Hash a password. Off the async runtime, because this is a tenth of a second of solid CPU and
/// holding a worker thread for it would stall every other request on a two-core machine.
async fn hash_password(password: String) -> Result<String, AuthError> {
    let _permit = hash_permits()
        .acquire()
        .await
        .map_err(|e| AuthError::Hashing(e.to_string()))?;
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        argon()?
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|e| AuthError::Hashing(e.to_string()))
    })
    .await
    .map_err(|e| AuthError::Hashing(e.to_string()))?
}

/// Verify a password against a stored hash. Returns false rather than an error for a wrong password,
/// because the caller must not be able to tell the difference between wrong and absent.
async fn verify_password(password: String, stored: String) -> bool {
    let Ok(permit) = hash_permits().acquire().await else {
        return false;
    };
    let result = tokio::task::spawn_blocking(move || {
        let Ok(parsed) = PasswordHash::new(&stored) else {
            return false;
        };
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    })
    .await;
    drop(permit);
    result.unwrap_or(false)
}

/// A hash of a password nobody has, used to spend the same time on an unknown address as on a known
/// one. Computed once; it is the shape of the work that matters, not the value.
fn decoy_hash() -> &'static str {
    static DECOY: OnceLock<String> = OnceLock::new();
    DECOY.get_or_init(|| {
        let salt = SaltString::generate(&mut OsRng);
        argon()
            .expect("argon parameters are constants")
            .hash_password(b"an address that does not exist", &salt)
            .expect("hashing a constant cannot fail")
            .to_string()
    })
}

fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// Create an account.
///
/// The owner account is the first one and is made on the machine, never through a web page. A
/// second owner is refused by a unique index rather than by this function remembering to check.
pub async fn create_account(
    pool: &Db,
    email: &str,
    display_name: &str,
    role: Role,
    password: Option<&str>,
) -> Result<Account, AuthError> {
    let email = normalize_email(email);
    if email.is_empty() || !email.contains('@') {
        return Err(AuthError::Invalid("that is not an email address".into()));
    }
    // Only the owner account exists in this milestone. An `agent` row would be an account that can
    // hold a session while nothing in the API checks roles, so creating one is refused rather than
    // left as a variant that looks supported. See docs/AUTHENTICATION.md.
    if role != Role::Owner {
        return Err(AuthError::Invalid(format!(
            "{} accounts are disabled in this build: no query filters by owner and no route checks a role, so any account but the owner's would read the owner's data. See docs/AUTHENTICATION.md.",
            role.as_str()
        )));
    }
    let hash = match password {
        Some(password) => {
            check_password_rules(password)?;
            Some(hash_password(password.to_owned()).await?)
        }
        None => {
            return Err(AuthError::Invalid(
                "an account that can log in needs a password".into(),
            ))
        }
    };

    let owner_id = crate::profiles::local_owner(pool)
        .await
        .map_err(|e| AuthError::Invalid(e.to_string()))?;
    let row = sqlx::query_as::<_, AccountRow>(&format!(
        "insert into users (owner_id, email, display_name, role, password_hash)
         values ($1,$2,$3,$4,$5) returning {ACCOUNT_COLUMNS}"
    ))
    .bind(owner_id)
    .bind(&email)
    .bind(display_name.trim())
    .bind(role.as_str())
    .bind(hash)
    .fetch_one(pool)
    .await
    .map_err(|error| match &error {
        sqlx::Error::Database(db) if db.is_unique_violation() => match db.constraint() {
            Some("users_single_owner") => AuthError::OwnerExists,
            _ => AuthError::DuplicateEmail,
        },
        _ => AuthError::Database(error),
    })?;
    row.account()
}

/// Replace an account's password and revoke every session it had, because a password is changed for
/// exactly two reasons and both of them mean the old sessions should stop.
pub async fn set_password(pool: &Db, email: &str, password: &str) -> Result<Account, AuthError> {
    check_password_rules(password)?;
    // Only an owner account has a password at all. Without this, setting one on a non-owner row
    // would quietly turn it into an account that logs in through the ordinary form while nothing
    // checks what it may do.
    let existing = sqlx::query_as::<_, AccountRow>(&format!(
        "select {ACCOUNT_COLUMNS} from users where lower(email) = lower($1)"
    ))
    .bind(normalize_email(email))
    .fetch_optional(pool)
    .await?
    .ok_or(AuthError::NoSuchAccount)?;
    if existing.account()?.role != Role::Owner {
        return Err(AuthError::Invalid(
            "only the owner account has a password in this build".into(),
        ));
    }
    let hash = hash_password(password.to_owned()).await?;
    let row = sqlx::query_as::<_, AccountRow>(&format!(
        "update users set password_hash = $2, updated_at = now()
         where lower(email) = lower($1) returning {ACCOUNT_COLUMNS}"
    ))
    .bind(normalize_email(email))
    .bind(hash)
    .fetch_optional(pool)
    .await?
    .ok_or(AuthError::NoSuchAccount)?;
    revoke_all_sessions(pool, row.id).await?;
    row.account()
}

pub async fn list_accounts(pool: &Db) -> Result<Vec<Account>, AuthError> {
    let rows = sqlx::query_as::<_, AccountRow>(&format!(
        "select {ACCOUNT_COLUMNS} from users order by created_at"
    ))
    .fetch_all(pool)
    .await?;
    rows.iter().map(AccountRow::account).collect()
}

/// Whether any account exists at all. A deployment with none is one nobody can log in to, which the
/// startup line says out loud rather than leaving to be discovered at the login page.
pub async fn account_count(pool: &Db) -> Result<i64, AuthError> {
    Ok(sqlx::query_scalar("select count(*) from users")
        .fetch_one(pool)
        .await?)
}

/// Check an address and password.
///
/// `None` covers every failure: no such address, wrong password, disabled account, an account with
/// no password. The caller cannot tell them apart, and neither can anyone watching the clock,
/// because a missing account is still charged one Argon2 verification.
pub async fn authenticate(pool: &Db, email: &str, password: &str) -> Option<Account> {
    if password.len() > MAX_PASSWORD_BYTES {
        return None;
    }
    let row = sqlx::query_as::<_, AccountRow>(&format!(
        "select {ACCOUNT_COLUMNS} from users where lower(email) = lower($1)"
    ))
    .bind(normalize_email(email))
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let stored = row
        .as_ref()
        .and_then(|row| row.password_hash.clone())
        .unwrap_or_else(|| decoy_hash().to_owned());
    let matched = verify_password(password.to_owned(), stored).await;

    match row {
        Some(row) if matched && row.disabled_at.is_none() => {
            let account = row.account().ok()?;
            // The deny rule. No route checks a role yet, so a session is a session: anything but
            // the owner is refused here, where there is exactly one place to get it wrong.
            if account.role != Role::Owner {
                tracing::warn!(
                    role = account.role.as_str(),
                    "refused a login by a non-owner account"
                );
                return None;
            }
            Some(account)
        }
        _ => None,
    }
}

/// Whether this account may hold a session at all. The same rule the login path applies, available
/// to anything that resolves a session rather than issues one.
pub fn may_hold_a_session(account: &Account) -> bool {
    account.role == Role::Owner && !account.disabled
}

/// An issued session: the token goes to the browser, nothing else ever sees it again.
pub struct IssuedSession {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

fn token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// Start a session. 256 bits of operating-system randomness, hex encoded; the database keeps only
/// its hash, so a copy of the table is not a set of working cookies.
pub async fn begin_session(pool: &Db, user_id: Uuid) -> Result<IssuedSession, AuthError> {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let expires_at = Utc::now() + Duration::hours(SESSION_TTL_HOURS);
    sqlx::query("insert into user_sessions (user_id, token_hash, expires_at) values ($1,$2,$3)")
        .bind(user_id)
        .bind(token_hash(&token))
        .bind(expires_at)
        .execute(pool)
        .await?;
    Ok(IssuedSession { token, expires_at })
}

/// Resolve a token to the account it belongs to, or nothing.
///
/// Expiry and revocation are checked in the query rather than afterwards, so there is no path that
/// reads the row and forgets to look at them.
/// Resolve a token to the account it belongs to, or nothing.
pub async fn session_account(pool: &Db, token: &str) -> Option<Account> {
    let row = sqlx::query_as::<_, AccountRow>(&format!(
        "select {}
         from user_sessions s join users u on u.id = s.user_id
         where s.token_hash = $1 and s.revoked_at is null and s.expires_at > now()
           and u.disabled_at is null",
        ACCOUNT_COLUMNS
            .split(", ")
            .map(|c| format!("u.{c}"))
            .collect::<Vec<_>>()
            .join(", ")
    ))
    .bind(token_hash(token))
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;

    let account = row.account().ok()?;
    // A session that predates the rule, or belongs to an account whose role changed, is refused
    // here as well as at login. The check lives in both places because a session outlives the login
    // that made it.
    if !may_hold_a_session(&account) {
        return None;
    }

    // Best effort, and only once a minute: this runs on every authenticated request, and writing a
    // row per request is write amplification bought for nothing.
    let _ = sqlx::query(
        "update user_sessions set last_seen_at = now()
         where token_hash = $1 and last_seen_at < now() - interval '1 minute'",
    )
    .bind(token_hash(token))
    .execute(pool)
    .await;
    Some(account)
}

/// End one session. Idempotent, because a logout that races a second tab is not an error.
pub async fn revoke_session(pool: &Db, token: &str) -> Result<(), AuthError> {
    sqlx::query(
        "update user_sessions set revoked_at = now() where token_hash = $1 and revoked_at is null",
    )
    .bind(token_hash(token))
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn revoke_all_sessions(pool: &Db, user_id: Uuid) -> Result<u64, AuthError> {
    let result = sqlx::query(
        "update user_sessions set revoked_at = now() where user_id = $1 and revoked_at is null",
    )
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Delete sessions that ended some time ago. Expiry is already enforced in the lookup; this only
/// stops the table growing for ever.
pub async fn purge_expired_sessions(pool: &Db) -> Result<u64, AuthError> {
    let result = sqlx::query(
        "delete from user_sessions
         where expires_at < now() - interval '7 days'
            or (revoked_at is not null and revoked_at < now() - interval '7 days')",
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_has_to_be_long_enough_and_bounded() {
        assert!(check_password_rules("short").is_err());
        assert!(check_password_rules("correct horse battery staple").is_ok());
        assert!(check_password_rules(&"x".repeat(MAX_PASSWORD_BYTES + 1)).is_err());
    }

    #[tokio::test]
    async fn a_hash_verifies_only_against_its_own_password() {
        let hash = hash_password("correct horse battery staple".into())
            .await
            .unwrap();
        // The stored form is a PHC string naming the algorithm, not the password in any form.
        assert!(hash.starts_with("$argon2id$"));
        assert!(!hash.contains("correct horse"));
        assert!(verify_password("correct horse battery staple".into(), hash.clone()).await);
        assert!(!verify_password("Correct horse battery staple".into(), hash.clone()).await);
        assert!(!verify_password(String::new(), hash).await);
    }

    #[tokio::test]
    async fn a_corrupt_stored_hash_refuses_rather_than_panicking() {
        assert!(!verify_password("whatever".into(), "not a phc string".into()).await);
    }
}
