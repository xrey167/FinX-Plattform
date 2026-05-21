#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    pub subject: String,
    pub roles: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthPolicy {
    pub table: String,
    pub required_role: String,
    pub row_filter: Option<String>,
}

pub fn authorize(principal: &Principal, policy: &AuthPolicy) -> bool {
    principal
        .roles
        .iter()
        .any(|role| role == &policy.required_role)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_policy_allows_and_denies_protected_query_paths() {
        let policy = AuthPolicy {
            table: "analytics.gold_daily_returns".to_string(),
            required_role: "analyst".to_string(),
            row_filter: Some("tenant_id = current_tenant()".to_string()),
        };
        assert!(authorize(
            &Principal {
                subject: "alice".to_string(),
                roles: vec!["analyst".to_string()],
            },
            &policy
        ));
        assert!(!authorize(
            &Principal {
                subject: "bob".to_string(),
                roles: vec!["guest".to_string()],
            },
            &policy
        ));
    }
}
