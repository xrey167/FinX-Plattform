#![forbid(unsafe_code)]
//! OIDC claim/JWKS **structural** validation for the daemon ingress boundary.
//!
//! This crate checks that a set of decoded [`JwtClaims`] is internally
//! consistent with a local [`JwksKey`] set and the expected issuer/audience:
//! non-empty subject, issuer/audience match, a non-empty `kid` that is present
//! in the JWKS, an allowed algorithm (default [`DEFAULT_ALLOWED_ALGORITHMS`] =
//! `RS256`/`ES256`), and well-formed role names. See [`validate_claims_strict`]
//! (rich [`ClaimValidationError`]) and the [`validate_claims`] bool shim.
//!
//! # Scope: structural, not cryptographic
//!
//! It does **not** verify JWT signatures, parse/decode tokens, fetch or refresh
//! a remote JWKS, or check token expiry — those are tracked follow-ups for the
//! auth-service adapter. Consumers (notably `tdw-service-api`'s production
//! `TDW_OIDC_*` policy builder) rely on this for claim/JWKS consistency only.
//! See `docs/release/production-auth-oidc.md`.

use serde::{Deserialize, Serialize};

pub const DEFAULT_ALLOWED_ALGORITHMS: [&str; 2] = ["RS256", "ES256"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JwksKey {
    pub kid: String,
    pub alg: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JwtClaims {
    pub sub: String,
    pub iss: String,
    pub aud: String,
    pub kid: String,
    pub roles: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimValidationError {
    EmptySubject,
    IssuerMismatch,
    AudienceMismatch,
    EmptyKeyId,
    UnknownKeyId,
    UnsupportedAlgorithm,
    InvalidRole,
}

#[must_use]
pub fn validate_claims(claims: &JwtClaims, jwks: &[JwksKey], issuer: &str, audience: &str) -> bool {
    validate_claims_strict(claims, jwks, issuer, audience, &DEFAULT_ALLOWED_ALGORITHMS).is_ok()
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_claims_strict(
    claims: &JwtClaims,
    jwks: &[JwksKey],
    issuer: &str,
    audience: &str,
    allowed_algorithms: &[&str],
) -> Result<(), ClaimValidationError> {
    if claims.sub.trim().is_empty() {
        return Err(ClaimValidationError::EmptySubject);
    }
    if claims.iss != issuer || issuer.trim().is_empty() {
        return Err(ClaimValidationError::IssuerMismatch);
    }
    if claims.aud != audience || audience.trim().is_empty() {
        return Err(ClaimValidationError::AudienceMismatch);
    }
    if claims.kid.trim().is_empty() {
        return Err(ClaimValidationError::EmptyKeyId);
    }
    if claims.roles.iter().any(|role| !is_role_name(role)) {
        return Err(ClaimValidationError::InvalidRole);
    }

    let Some(key) = jwks.iter().find(|key| key.kid == claims.kid) else {
        return Err(ClaimValidationError::UnknownKeyId);
    };
    if !allowed_algorithms.iter().any(|alg| *alg == key.alg) {
        return Err(ClaimValidationError::UnsupportedAlgorithm);
    }

    Ok(())
}

fn is_role_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_jwt_claims_against_jwks_issuer_and_audience() {
        let key = JwksKey {
            kid: "k1".to_string(),
            alg: "RS256".to_string(),
        };
        let claims = JwtClaims {
            sub: "alice".to_string(),
            iss: "https://issuer".to_string(),
            aud: "tdw".to_string(),
            kid: "k1".to_string(),
            roles: vec!["analyst".to_string()],
        };

        assert!(validate_claims(&claims, &[key], "https://issuer", "tdw"));
        assert!(!validate_claims(&claims, &[], "https://issuer", "tdw"));
    }

    #[test]
    fn rejects_empty_subject_unknown_algorithms_and_bad_roles() {
        let key = JwksKey {
            kid: "k1".to_string(),
            alg: "none".to_string(),
        };
        let claims = JwtClaims {
            sub: "alice".to_string(),
            iss: "https://issuer".to_string(),
            aud: "tdw".to_string(),
            kid: "k1".to_string(),
            roles: vec!["analyst".to_string()],
        };

        assert_eq!(
            validate_claims_strict(&claims, &[key], "https://issuer", "tdw", &["RS256"]),
            Err(ClaimValidationError::UnsupportedAlgorithm)
        );
        assert_eq!(
            validate_claims_strict(
                &JwtClaims {
                    sub: " ".to_string(),
                    ..claims.clone()
                },
                &[JwksKey {
                    kid: "k1".to_string(),
                    alg: "RS256".to_string(),
                }],
                "https://issuer",
                "tdw",
                &["RS256"],
            ),
            Err(ClaimValidationError::EmptySubject)
        );
        assert_eq!(
            validate_claims_strict(
                &JwtClaims {
                    roles: vec!["analyst\nadmin".to_string()],
                    ..claims
                },
                &[JwksKey {
                    kid: "k1".to_string(),
                    alg: "RS256".to_string(),
                }],
                "https://issuer",
                "tdw",
                &["RS256"],
            ),
            Err(ClaimValidationError::InvalidRole)
        );
    }
}
