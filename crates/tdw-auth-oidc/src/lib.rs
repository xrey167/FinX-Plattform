#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

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

pub fn validate_claims(claims: &JwtClaims, jwks: &[JwksKey], issuer: &str, audience: &str) -> bool {
    claims.iss == issuer && claims.aud == audience && jwks.iter().any(|key| key.kid == claims.kid)
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
}
