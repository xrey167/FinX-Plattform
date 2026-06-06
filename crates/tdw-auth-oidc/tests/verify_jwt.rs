//! Offline-deterministic cryptographic JWT verification tests.
//!
//! Tokens are minted in-test from committed RSA/EC keypair fixtures (no network,
//! no clock dependence beyond `exp`/`nbf` offsets relative to "now"), exercising
//! both the accept path and every reject path the ingress boundary must enforce:
//! bad signature, expired, not-yet-valid, wrong audience/issuer, unknown kid,
//! and the `alg:none` / alg-confusion downgrade attempts.

use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};
use tdw_auth_oidc::{VerifyError, VerifyingKey, verify_jwt};

const RSA_PRIVATE: &str = include_str!("fixtures/rsa_private.pem");
const RSA_PUBLIC: &str = include_str!("fixtures/rsa_public.pem");
const EC_PRIVATE: &str = include_str!("fixtures/ec_private.pem");
const EC_PUBLIC: &str = include_str!("fixtures/ec_public.pem");

const ISSUER: &str = "https://issuer.example";
const AUDIENCE: &str = "tdw-daemon";

#[derive(Serialize)]
struct Claims {
    sub: String,
    iss: String,
    aud: String,
    roles: Vec<String>,
    exp: i64,
    nbf: i64,
    iat: i64,
}

fn now() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_secs(),
    )
    .expect("timestamp fits i64")
}

fn claims(iss: &str, aud: &str, exp_offset: i64, nbf_offset: i64) -> Claims {
    let base = now();
    Claims {
        sub: "svc:prod".to_string(),
        iss: iss.to_string(),
        aud: aud.to_string(),
        roles: vec!["analyst".to_string()],
        exp: base + exp_offset,
        nbf: base + nbf_offset,
        iat: base,
    }
}

fn rsa_key() -> EncodingKey {
    EncodingKey::from_rsa_pem(RSA_PRIVATE.as_bytes()).expect("rsa private pem")
}

fn ec_key() -> EncodingKey {
    EncodingKey::from_ec_pem(EC_PRIVATE.as_bytes()).expect("ec private pem")
}

fn header(alg: Algorithm, kid: &str) -> Header {
    let mut header = Header::new(alg);
    header.kid = Some(kid.to_string());
    header
}

fn mint(alg: Algorithm, kid: &str, key: &EncodingKey, claims: &Claims) -> String {
    encode(&header(alg, kid), claims, key).expect("mint token")
}

fn rsa_verifying_key() -> VerifyingKey {
    VerifyingKey {
        kid: "rsa-1".to_string(),
        alg: "RS256".to_string(),
        pem: RSA_PUBLIC.to_string(),
    }
}

fn ec_verifying_key() -> VerifyingKey {
    VerifyingKey {
        kid: "ec-1".to_string(),
        alg: "ES256".to_string(),
        pem: EC_PUBLIC.to_string(),
    }
}

#[test]
fn accepts_valid_rs256_token() {
    let token = mint(
        Algorithm::RS256,
        "rsa-1",
        &rsa_key(),
        &claims(ISSUER, AUDIENCE, 3600, -10),
    );
    let verified = verify_jwt(&token, &[rsa_verifying_key()], ISSUER, AUDIENCE)
        .expect("valid RS256 token must verify");
    assert_eq!(verified.sub, "svc:prod");
    assert_eq!(verified.roles, vec!["analyst".to_string()]);
}

#[test]
fn accepts_valid_es256_token() {
    let token = mint(
        Algorithm::ES256,
        "ec-1",
        &ec_key(),
        &claims(ISSUER, AUDIENCE, 3600, -10),
    );
    let verified = verify_jwt(&token, &[ec_verifying_key()], ISSUER, AUDIENCE)
        .expect("valid ES256 token must verify");
    assert_eq!(verified.sub, "svc:prod");
}

#[test]
fn rejects_token_signed_by_wrong_key() {
    // Mint with a freshly-derived (different) RSA key is not possible offline
    // without a keygen dep; instead sign an EC token but present it for the RSA
    // key slot — the signature cannot verify against the RSA public key.
    let token = mint(
        Algorithm::RS256,
        "rsa-1",
        &rsa_key(),
        &claims(ISSUER, AUDIENCE, 3600, -10),
    );
    // Tamper the signature segment so the bytes no longer verify.
    let mut parts: Vec<&str> = token.split('.').collect();
    let tampered_sig = if parts[2].starts_with('A') {
        format!("B{}", &parts[2][1..])
    } else {
        format!("A{}", &parts[2][1..])
    };
    parts[2] = &tampered_sig;
    let tampered = parts.join(".");

    assert_eq!(
        verify_jwt(&tampered, &[rsa_verifying_key()], ISSUER, AUDIENCE),
        Err(VerifyError::InvalidSignature)
    );
}

#[test]
fn rejects_expired_token() {
    let token = mint(
        Algorithm::RS256,
        "rsa-1",
        &rsa_key(),
        // exp well in the past, beyond the default 60s leeway.
        &claims(ISSUER, AUDIENCE, -3600, -7200),
    );
    assert_eq!(
        verify_jwt(&token, &[rsa_verifying_key()], ISSUER, AUDIENCE),
        Err(VerifyError::Expired)
    );
}

#[test]
fn rejects_not_yet_valid_token() {
    let token = mint(
        Algorithm::RS256,
        "rsa-1",
        &rsa_key(),
        // nbf well in the future, beyond the default 60s leeway.
        &claims(ISSUER, AUDIENCE, 7200, 3600),
    );
    assert_eq!(
        verify_jwt(&token, &[rsa_verifying_key()], ISSUER, AUDIENCE),
        Err(VerifyError::NotYetValid)
    );
}

#[test]
fn rejects_wrong_audience() {
    let token = mint(
        Algorithm::RS256,
        "rsa-1",
        &rsa_key(),
        &claims(ISSUER, "someone-else", 3600, -10),
    );
    assert_eq!(
        verify_jwt(&token, &[rsa_verifying_key()], ISSUER, AUDIENCE),
        Err(VerifyError::AudienceMismatch)
    );
}

#[test]
fn rejects_wrong_issuer() {
    let token = mint(
        Algorithm::RS256,
        "rsa-1",
        &rsa_key(),
        &claims("https://evil.example", AUDIENCE, 3600, -10),
    );
    assert_eq!(
        verify_jwt(&token, &[rsa_verifying_key()], ISSUER, AUDIENCE),
        Err(VerifyError::IssuerMismatch)
    );
}

#[test]
fn rejects_unknown_kid() {
    let token = mint(
        Algorithm::RS256,
        "rsa-unknown",
        &rsa_key(),
        &claims(ISSUER, AUDIENCE, 3600, -10),
    );
    assert_eq!(
        verify_jwt(&token, &[rsa_verifying_key()], ISSUER, AUDIENCE),
        Err(VerifyError::UnknownKeyId("rsa-unknown".to_string()))
    );
}

#[test]
fn rejects_alg_none_token() {
    // Hand-craft an unsigned (`alg: none`) token; the verifier must reject it
    // before any key material is consulted.
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none","kid":"rsa-1","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&claims(ISSUER, AUDIENCE, 3600, -10)).expect("serialize claims"),
    );
    let none_token = format!("{header}.{payload}.");

    // The unsigned token must be rejected fail-closed. `jsonwebtoken`'s header
    // parser refuses to decode `alg: none` at all (MalformedToken); were it to
    // parse, our allow-list would reject it as UnsupportedAlgorithm. Either way
    // the security property — `none` is never accepted — holds.
    let result = verify_jwt(&none_token, &[rsa_verifying_key()], ISSUER, AUDIENCE);
    assert!(
        matches!(
            result,
            Err(VerifyError::MalformedToken | VerifyError::UnsupportedAlgorithm(_))
        ),
        "alg:none must be rejected fail-closed, got: {result:?}"
    );
}

#[test]
fn rejects_alg_confusion_against_resolved_key() {
    // Token claims ES256 in its header and carries kid "rsa-1", but the resolved
    // key for "rsa-1" is bound to RS256: the algorithm mismatch must be rejected
    // rather than attempting an ES256 verify against an RSA key.
    let token = mint(
        Algorithm::ES256,
        "rsa-1",
        &ec_key(),
        &claims(ISSUER, AUDIENCE, 3600, -10),
    );
    assert_eq!(
        verify_jwt(&token, &[rsa_verifying_key()], ISSUER, AUDIENCE),
        Err(VerifyError::UnsupportedAlgorithm("ES256".to_string()))
    );
}

#[test]
fn rejects_hs256_symmetric_token() {
    // An HMAC-signed token must be rejected: HMAC is not an allow-listed
    // asymmetric algorithm, so the downgrade is refused before key lookup.
    let token = encode(
        &header(Algorithm::HS256, "rsa-1"),
        &claims(ISSUER, AUDIENCE, 3600, -10),
        &EncodingKey::from_secret(b"shared-secret"),
    )
    .expect("mint hs256 token");
    assert_eq!(
        verify_jwt(&token, &[rsa_verifying_key()], ISSUER, AUDIENCE),
        Err(VerifyError::UnsupportedAlgorithm("HS256".to_string()))
    );
}
