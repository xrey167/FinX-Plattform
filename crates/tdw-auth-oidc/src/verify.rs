//! Cryptographic JWT verification for the OIDC ingress boundary.
//!
//! This is the cryptographic counterpart to the structural
//! [`validate_claims_strict`](crate::validate_claims_strict) pre-filter: given a
//! raw compact JWT, a set of verifying keys ([`VerifyingKey`]), and the expected
//! issuer/audience, [`verify_jwt`] verifies the token signature against the key
//! whose `kid` matches the token header, then enforces `exp`/`nbf`/`iat` (with a
//! small clock skew), issuer, and audience — failing closed on any error.
//!
//! Built on [`jsonwebtoken`] (default `ring` backend). Only the asymmetric
//! algorithms the platform allows are accepted (`RS256`/`ES256` by default); the
//! `none` algorithm and HMAC algorithms are rejected before any key lookup, so a
//! token cannot downgrade itself to an unauthenticated or symmetric path
//! (alg-confusion / `alg:none` defence).

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::{Deserialize, Serialize};

/// Default clock-skew leeway, in seconds, applied to `exp`/`nbf`/`iat`.
pub const DEFAULT_LEEWAY_SECS: u64 = 60;

/// A verifying (public) key usable for JWT signature checks.
///
/// `pem` carries the PEM-encoded SubjectPublicKeyInfo (for RSA) or the
/// SEC1/PKCS#8 public key (for EC) the IdP publishes for `kid`. `alg` is the
/// JWS algorithm the key is bound to (e.g. `RS256`, `ES256`); a token whose
/// header algorithm does not match the resolved key's `alg` is rejected.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyingKey {
    pub kid: String,
    pub alg: String,
    pub pem: String,
}

/// Standard JWT claims the verifier deserializes and returns on success.
///
/// `exp`/`nbf`/`iat` are validated by [`jsonwebtoken`] itself; `iss`/`aud` are
/// validated against the caller-supplied expected values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedClaims {
    pub sub: String,
    pub iss: String,
    #[serde(default)]
    pub aud: String,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub exp: Option<i64>,
    #[serde(default)]
    pub nbf: Option<i64>,
    #[serde(default)]
    pub iat: Option<i64>,
}

/// A cryptographic verification failure. Every variant is fail-closed: the
/// caller must reject the request on any `Err`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyError {
    /// The compact token or its header could not be parsed.
    MalformedToken,
    /// The header carried no `kid`, so the signing key cannot be resolved.
    MissingKeyId,
    /// No supplied [`VerifyingKey`] matched the token's `kid`.
    UnknownKeyId(String),
    /// The header algorithm is not in the allow-list (covers `none`, HMAC, and
    /// any other non-permitted algorithm — the alg-confusion / `alg:none`
    /// defence), or it disagrees with the resolved key's bound algorithm.
    UnsupportedAlgorithm(String),
    /// The configured verifying key PEM could not be parsed for its algorithm.
    InvalidKey,
    /// The signature did not verify against the resolved key.
    InvalidSignature,
    /// The token is expired (`exp` in the past, beyond leeway).
    Expired,
    /// The token is not yet valid (`nbf`/`iat` in the future, beyond leeway).
    NotYetValid,
    /// The `iss` claim did not match the expected issuer.
    IssuerMismatch,
    /// The `aud` claim did not match the expected audience.
    AudienceMismatch,
    /// A required standard claim was absent.
    MissingClaim,
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedToken => write!(f, "malformed token"),
            Self::MissingKeyId => write!(f, "token header missing kid"),
            Self::UnknownKeyId(kid) => write!(f, "no verifying key for kid: {kid}"),
            Self::UnsupportedAlgorithm(alg) => write!(f, "unsupported algorithm: {alg}"),
            Self::InvalidKey => write!(f, "verifying key could not be parsed"),
            Self::InvalidSignature => write!(f, "signature verification failed"),
            Self::Expired => write!(f, "token expired"),
            Self::NotYetValid => write!(f, "token not yet valid"),
            Self::IssuerMismatch => write!(f, "issuer mismatch"),
            Self::AudienceMismatch => write!(f, "audience mismatch"),
            Self::MissingClaim => write!(f, "missing required claim"),
        }
    }
}

impl std::error::Error for VerifyError {}

/// Map a JWS algorithm name to the [`jsonwebtoken::Algorithm`] enum, accepting
/// only the asymmetric algorithms in `allowed`. Returns `None` for `none`,
/// HMAC, or any algorithm not present in `allowed`.
fn resolve_algorithm(alg: &str, allowed: &[&str]) -> Option<Algorithm> {
    if !allowed.contains(&alg) {
        return None;
    }
    match alg {
        "RS256" => Some(Algorithm::RS256),
        "RS384" => Some(Algorithm::RS384),
        "RS512" => Some(Algorithm::RS512),
        "ES256" => Some(Algorithm::ES256),
        "ES384" => Some(Algorithm::ES384),
        "PS256" => Some(Algorithm::PS256),
        "PS384" => Some(Algorithm::PS384),
        "PS512" => Some(Algorithm::PS512),
        // `EdDSA`, HMAC (`HS*`) and the `none` pseudo-algorithm are intentionally
        // unmapped: the platform does not accept symmetric or unsigned tokens at
        // the ingress boundary.
        _ => None,
    }
}

/// Build a [`DecodingKey`] from a [`VerifyingKey`]'s PEM for the given
/// algorithm. RSA-family algorithms expect an RSA SPKI PEM; EC algorithms
/// expect an EC public-key PEM.
fn decoding_key(pem: &str, algorithm: Algorithm) -> Result<DecodingKey, VerifyError> {
    let bytes = pem.as_bytes();
    let key = match algorithm {
        Algorithm::RS256
        | Algorithm::RS384
        | Algorithm::RS512
        | Algorithm::PS256
        | Algorithm::PS384
        | Algorithm::PS512 => DecodingKey::from_rsa_pem(bytes),
        Algorithm::ES256 | Algorithm::ES384 => DecodingKey::from_ec_pem(bytes),
        // Unreachable for allow-listed algorithms (see `resolve_algorithm`).
        _ => return Err(VerifyError::UnsupportedAlgorithm(format!("{algorithm:?}"))),
    };
    key.map_err(|_| VerifyError::InvalidKey)
}

/// Verify a compact JWT cryptographically and validate its standard claims,
/// using [`DEFAULT_ALLOWED_ALGORITHMS`](crate::DEFAULT_ALLOWED_ALGORITHMS) and a
/// [`DEFAULT_LEEWAY_SECS`] clock skew.
///
/// See [`verify_jwt_strict`] for the configurable form.
///
/// # Errors
///
/// Returns a [`VerifyError`] on any parse, key-resolution, signature, or claim
/// failure (fail closed).
pub fn verify_jwt(
    token: &str,
    keys: &[VerifyingKey],
    issuer: &str,
    audience: &str,
) -> Result<VerifiedClaims, VerifyError> {
    verify_jwt_strict(
        token,
        keys,
        issuer,
        audience,
        &crate::DEFAULT_ALLOWED_ALGORITHMS,
        DEFAULT_LEEWAY_SECS,
    )
}

/// Verify a compact JWT cryptographically and validate its standard claims.
///
/// Steps (fail-closed at every stage):
/// 1. Parse the JWS header; require a non-empty `kid`.
/// 2. Resolve the header algorithm against `allowed_algorithms`, rejecting
///    `none`/HMAC/unlisted algorithms before any key material is touched.
/// 3. Resolve the verifying key by `kid` and require its bound `alg` to equal
///    the header algorithm (no alg confusion across keys).
/// 4. Verify the signature and enforce `exp`/`nbf`/`iat` (with `leeway_secs`
///    clock skew), `iss`, and `aud`.
///
/// # Errors
///
/// Returns a [`VerifyError`] on any parse, key-resolution, signature, or claim
/// failure.
pub fn verify_jwt_strict(
    token: &str,
    keys: &[VerifyingKey],
    issuer: &str,
    audience: &str,
    allowed_algorithms: &[&str],
    leeway_secs: u64,
) -> Result<VerifiedClaims, VerifyError> {
    let header = decode_header(token).map_err(|_| VerifyError::MalformedToken)?;
    let kid = header
        .kid
        .filter(|kid| !kid.trim().is_empty())
        .ok_or(VerifyError::MissingKeyId)?;

    let header_alg = algorithm_name(header.alg);
    let algorithm = resolve_algorithm(header_alg, allowed_algorithms)
        .ok_or_else(|| VerifyError::UnsupportedAlgorithm(header_alg.to_string()))?;

    let key = keys
        .iter()
        .find(|key| key.kid == kid)
        .ok_or_else(|| VerifyError::UnknownKeyId(kid.clone()))?;

    // The resolved key must be bound to the same algorithm the header claims:
    // this prevents a token from selecting a key intended for a different
    // algorithm (alg confusion across the JWKS).
    if key.alg != header_alg {
        return Err(VerifyError::UnsupportedAlgorithm(header_alg.to_string()));
    }

    let decoding = decoding_key(&key.pem, algorithm)?;

    let mut validation = Validation::new(algorithm);
    validation.leeway = leeway_secs;
    validation.validate_exp = true;
    validation.validate_nbf = true;
    validation.set_issuer(&[issuer]);
    validation.set_audience(&[audience]);
    // Only the explicitly-listed algorithm is accepted by the underlying decode.
    validation.algorithms = vec![algorithm];

    decode::<VerifiedClaims>(token, &decoding, &validation)
        .map(|data| data.claims)
        .map_err(map_jwt_error)
}

/// Map a [`jsonwebtoken`] error kind onto our fail-closed [`VerifyError`].
fn map_jwt_error(error: jsonwebtoken::errors::Error) -> VerifyError {
    use jsonwebtoken::errors::ErrorKind;
    match error.kind() {
        ErrorKind::ExpiredSignature => VerifyError::Expired,
        ErrorKind::ImmatureSignature => VerifyError::NotYetValid,
        ErrorKind::InvalidIssuer => VerifyError::IssuerMismatch,
        ErrorKind::InvalidAudience => VerifyError::AudienceMismatch,
        ErrorKind::InvalidSignature => VerifyError::InvalidSignature,
        ErrorKind::MissingRequiredClaim(_) => VerifyError::MissingClaim,
        ErrorKind::InvalidAlgorithm | ErrorKind::InvalidAlgorithmName => {
            VerifyError::UnsupportedAlgorithm(String::new())
        }
        ErrorKind::InvalidRsaKey(_) | ErrorKind::InvalidEcdsaKey | ErrorKind::InvalidKeyFormat => {
            VerifyError::InvalidKey
        }
        // Any remaining structural/decoding failure is a malformed token.
        _ => VerifyError::MalformedToken,
    }
}

/// The canonical JWS name for a [`jsonwebtoken::Algorithm`].
fn algorithm_name(algorithm: Algorithm) -> &'static str {
    match algorithm {
        Algorithm::HS256 => "HS256",
        Algorithm::HS384 => "HS384",
        Algorithm::HS512 => "HS512",
        Algorithm::ES256 => "ES256",
        Algorithm::ES384 => "ES384",
        Algorithm::RS256 => "RS256",
        Algorithm::RS384 => "RS384",
        Algorithm::RS512 => "RS512",
        Algorithm::PS256 => "PS256",
        Algorithm::PS384 => "PS384",
        Algorithm::PS512 => "PS512",
        Algorithm::EdDSA => "EdDSA",
    }
}
