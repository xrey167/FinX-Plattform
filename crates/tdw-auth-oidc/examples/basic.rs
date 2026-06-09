//! Offline `tdw-auth-oidc` example: validate a set of self-minted JWT claims
//! against an inline JWKS, mirroring the crate's own tests.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p tdw-auth-oidc --example tdw_auth_oidc_basic
//! ```
//!
//! Note: this crate performs *structural* validation only — it checks that the
//! decoded claims are consistent with the JWKS and the expected issuer/audience.
//! It does not verify a JWT signature (that is a tracked follow-up in the
//! auth-service adapter). The "self-minted token" here is therefore the decoded
//! `JwtClaims` value, not a signed compact JWT string.

#![forbid(unsafe_code)]

use tdw_auth_oidc::{
    ClaimValidationError, DEFAULT_ALLOWED_ALGORITHMS, JwksKey, JwtClaims, validate_claims,
    validate_claims_strict,
};

fn main() {
    // An inline JWKS with a single RS256 key.
    let jwks = [JwksKey {
        kid: "k1".to_string(),
        alg: "RS256".to_string(),
    }];

    let issuer = "https://issuer.example";
    let audience = "tdw";

    // A well-formed claim set referencing that key.
    let claims = JwtClaims {
        sub: "alice".to_string(),
        iss: issuer.to_string(),
        aud: audience.to_string(),
        kid: "k1".to_string(),
        roles: vec!["analyst".to_string()],
    };

    // Happy path: consistent claims validate.
    assert!(validate_claims(&claims, &jwks, issuer, audience));
    println!("valid claims accepted for sub={}", claims.sub);

    // Fail closed: an empty JWKS can never match a kid.
    assert!(!validate_claims(&claims, &[], issuer, audience));
    println!("empty JWKS rejected (fail-closed)");

    // Algorithm allow-list: a key advertising `none` is rejected even though the
    // kid matches.
    let none_jwks = [JwksKey {
        kid: "k1".to_string(),
        alg: "none".to_string(),
    }];
    assert_eq!(
        validate_claims_strict(
            &claims,
            &none_jwks,
            issuer,
            audience,
            &DEFAULT_ALLOWED_ALGORITHMS
        ),
        Err(ClaimValidationError::UnsupportedAlgorithm),
    );
    println!("alg=none rejected (allow-list)");

    // Injection guard: a role carrying a control character is rejected.
    let bad_roles = JwtClaims {
        roles: vec!["analyst\nadmin".to_string()],
        ..claims
    };
    assert_eq!(
        validate_claims_strict(
            &bad_roles,
            &jwks,
            issuer,
            audience,
            &DEFAULT_ALLOWED_ALGORITHMS
        ),
        Err(ClaimValidationError::InvalidRole),
    );
    println!("malformed role rejected");
}
