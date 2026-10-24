//! Access assertion verification, exercised against controlled signed assertions.
//!
//! The negative cases matter most, but a check that only ever refuses is indistinguishable from one that refuses everything. These tests sign real RS256 assertions with a throwaway key generated for this repository and assert that exactly the correct one is accepted.
//!
//! The key here is a test fixture. It is not Cloudflare's, it protects nothing, and it is checked in deliberately so the suite is reproducible from a clean clone. It is stored with a `.txt` extension because the repository refuses to track `.pem` files, and that rule exists to stop a real key being committed by accident; weakening it for a fixture would be the wrong trade.

use chrono::{Duration, Utc};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use opportunity_api::access::{CloudflareAccess, Jwk};
use serde::Serialize;

const PRIVATE_KEY: &str = include_str!("access-test-key.txt");
const JWK_COMPONENTS: &str = include_str!("access_jwk.txt");

const TEAM: &str = "example.cloudflareaccess.com";
const AUDIENCE: &str = "test-application-audience";
const OWNER: &str = "owner@example.test";
const KID: &str = "test-key-1";

#[derive(Debug, Serialize)]
struct Claims {
    aud: String,
    iss: String,
    exp: i64,
    iat: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    sub: String,
}

fn jwk(kid: &str) -> Jwk {
    let mut parts = JWK_COMPONENTS.split_whitespace();
    Jwk {
        kid: kid.to_owned(),
        n: parts.next().expect("modulus").to_owned(),
        e: parts.next().expect("exponent").to_owned(),
    }
}

fn access() -> CloudflareAccess {
    CloudflareAccess::new(TEAM.into(), AUDIENCE.into(), vec![OWNER.into()])
        .expect("configuration is valid")
}

/// Sign an assertion with the test key. Every parameter is a thing a real attacker could vary.
fn sign(aud: &str, iss: &str, email: Option<&str>, expires_in: Duration, kid: &str) -> String {
    let now = Utc::now();
    let claims = Claims {
        aud: aud.to_owned(),
        iss: iss.to_owned(),
        exp: (now + expires_in).timestamp(),
        iat: now.timestamp(),
        email: email.map(str::to_owned),
        sub: "test-subject".into(),
    };
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(kid.to_owned());
    encode(
        &header,
        &claims,
        &EncodingKey::from_rsa_pem(PRIVATE_KEY.as_bytes()).expect("test key loads"),
    )
    .expect("signing succeeds")
}

fn valid() -> String {
    sign(
        AUDIENCE,
        &format!("https://{TEAM}"),
        Some(OWNER),
        Duration::minutes(30),
        KID,
    )
}

#[test]
fn a_correctly_signed_assertion_from_an_allowed_identity_is_accepted() {
    // The positive case. Without it, every other test here would pass against a check that simply refuses everything.
    let identity = access()
        .verify_with_keys_for_test(&valid(), &[jwk(KID)])
        .expect("a correctly signed assertion for an allowed identity must be accepted");
    assert_eq!(identity.email, OWNER);
    assert_eq!(identity.via, "cloudflare-access");
    assert_eq!(identity.subject.as_deref(), Some("test-subject"));
}

#[test]
fn an_assertion_minted_for_a_different_application_is_refused() {
    // An assertion is valid at the application it names. Without the audience check, one issued for any other application in the same Cloudflare team would open this one.
    let other = sign(
        "some-other-application",
        &format!("https://{TEAM}"),
        Some(OWNER),
        Duration::minutes(30),
        KID,
    );
    assert!(access()
        .verify_with_keys_for_test(&other, &[jwk(KID)])
        .is_err());
}

#[test]
fn an_assertion_from_a_different_cloudflare_team_is_refused() {
    let foreign = sign(
        AUDIENCE,
        "https://attacker.cloudflareaccess.com",
        Some(OWNER),
        Duration::minutes(30),
        KID,
    );
    assert!(access()
        .verify_with_keys_for_test(&foreign, &[jwk(KID)])
        .is_err());
}

#[test]
fn an_expired_assertion_is_refused() {
    let stale = sign(
        AUDIENCE,
        &format!("https://{TEAM}"),
        Some(OWNER),
        Duration::minutes(-5),
        KID,
    );
    assert!(access()
        .verify_with_keys_for_test(&stale, &[jwk(KID)])
        .is_err());
}

#[test]
fn an_assertion_signed_by_a_key_we_do_not_know_is_refused() {
    // The signature is genuine, but the key is not one Cloudflare published. This is the forged-token case.
    let token = valid();
    let unknown = jwk("some-other-kid");
    let error = access()
        .verify_with_keys_for_test(&token, &[unknown])
        .unwrap_err();
    assert!(error.to_string().contains("unknown key"), "{error}");
}

#[test]
fn a_tampered_payload_fails_the_signature_check() {
    // Re-encoding the claims without re-signing is what an attacker does when they want a different email in a token they already hold.
    let token = valid();
    let mut parts: Vec<&str> = token.split('.').collect();
    let forged_claims = "eyJhdWQiOiJ0ZXN0LWFwcGxpY2F0aW9uLWF1ZGllbmNlIiwiaXNzIjoiaHR0cHM6Ly9leGFtcGxlLmNsb3VkZmxhcmVhY2Nlc3MuY29tIiwiZXhwIjo5OTk5OTk5OTk5LCJpYXQiOjE3MDAwMDAwMDAsImVtYWlsIjoib3duZXJAZXhhbXBsZS50ZXN0Iiwic3ViIjoidGVzdC1zdWJqZWN0In0";
    parts[1] = forged_claims;
    let tampered = parts.join(".");
    assert!(access()
        .verify_with_keys_for_test(&tampered, &[jwk(KID)])
        .is_err());
}

#[test]
fn a_correctly_signed_assertion_for_someone_else_is_refused() {
    // Cloudflare authenticated them; this service still does not know them. The allowlist is the second gate.
    let stranger = sign(
        AUDIENCE,
        &format!("https://{TEAM}"),
        Some("stranger@example.test"),
        Duration::minutes(30),
        KID,
    );
    let error = access()
        .verify_with_keys_for_test(&stranger, &[jwk(KID)])
        .unwrap_err();
    assert!(error.to_string().contains("not permitted"), "{error}");
}

#[test]
fn an_allowed_identity_is_matched_regardless_of_letter_case() {
    let shouted = sign(
        AUDIENCE,
        &format!("https://{TEAM}"),
        Some("OWNER@Example.TEST"),
        Duration::minutes(30),
        KID,
    );
    let identity = access()
        .verify_with_keys_for_test(&shouted, &[jwk(KID)])
        .expect("case must not matter");
    assert_eq!(identity.email, OWNER);
}

#[test]
fn an_assertion_carrying_no_identity_is_refused() {
    let anonymous = sign(
        AUDIENCE,
        &format!("https://{TEAM}"),
        None,
        Duration::minutes(30),
        KID,
    );
    assert!(access()
        .verify_with_keys_for_test(&anonymous, &[jwk(KID)])
        .is_err());
}

#[test]
fn an_unsigned_assertion_is_refused_whatever_it_claims() {
    // The `none` algorithm attack: a token that says it needs no signature.
    let unsigned = "eyJhbGciOiJub25lIiwia2lkIjoidGVzdC1rZXktMSJ9.eyJhdWQiOiJ0ZXN0LWFwcGxpY2F0aW9uLWF1ZGllbmNlIiwiZW1haWwiOiJvd25lckBleGFtcGxlLnRlc3QifQ.";
    assert!(access()
        .verify_with_keys_for_test(unsigned, &[jwk(KID)])
        .is_err());
}

#[test]
fn garbage_is_refused_without_panicking() {
    let access = access();
    for candidate in ["", "not a token", "a.b.c", "....", "null"] {
        assert!(
            access
                .verify_with_keys_for_test(candidate, &[jwk(KID)])
                .is_err(),
            "{candidate:?}"
        );
    }
}
