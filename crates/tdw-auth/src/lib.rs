#![forbid(unsafe_code)]

#![deny(clippy::pedantic, clippy::nursery)]
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorizationDecision {
    Allow,
    Deny(AuthorizationDenyReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorizationDenyReason {
    EmptySubject,
    InvalidPolicyTable,
    EmptyRequiredRole,
    InvalidRole,
    MissingRequiredRole,
}

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

#[must_use]
pub fn authorize(principal: &Principal, policy: &AuthPolicy) -> bool {
    authorize_with_decision(principal, policy) == AuthorizationDecision::Allow
}

#[must_use]
pub fn authorize_with_decision(
    principal: &Principal,
    policy: &AuthPolicy,
) -> AuthorizationDecision {
    if principal.subject.trim().is_empty() {
        return AuthorizationDecision::Deny(AuthorizationDenyReason::EmptySubject);
    }
    if !is_table_path(&policy.table) {
        return AuthorizationDecision::Deny(AuthorizationDenyReason::InvalidPolicyTable);
    }
    if policy.required_role.trim().is_empty() {
        return AuthorizationDecision::Deny(AuthorizationDenyReason::EmptyRequiredRole);
    }
    if principal.roles.iter().any(|role| !is_role_name(role))
        || !is_role_name(&policy.required_role)
    {
        return AuthorizationDecision::Deny(AuthorizationDenyReason::InvalidRole);
    }

    if principal
        .roles
        .iter()
        .any(|role| role == &policy.required_role)
    {
        AuthorizationDecision::Allow
    } else {
        AuthorizationDecision::Deny(AuthorizationDenyReason::MissingRequiredRole)
    }
}

fn is_role_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

fn is_table_path(value: &str) -> bool {
    let mut parts = value.split('.');
    let Some(schema) = parts.next() else {
        return false;
    };
    let Some(table) = parts.next() else {
        return false;
    };
    parts.next().is_none() && is_table_part(schema) && is_table_part(table)
}

fn is_table_part(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
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

    #[test]
    fn rejects_empty_subject_unsafe_table_and_invalid_roles() {
        let policy = AuthPolicy {
            table: "analytics.gold_daily_returns".to_string(),
            required_role: "analyst".to_string(),
            row_filter: None,
        };

        assert_eq!(
            authorize_with_decision(
                &Principal {
                    subject: " ".to_string(),
                    roles: vec!["analyst".to_string()],
                },
                &policy
            ),
            AuthorizationDecision::Deny(AuthorizationDenyReason::EmptySubject)
        );
        assert_eq!(
            authorize_with_decision(
                &Principal {
                    subject: "alice".to_string(),
                    roles: vec!["analyst".to_string()],
                },
                &AuthPolicy {
                    table: "analytics.gold_daily_returns;drop".to_string(),
                    ..policy.clone()
                },
            ),
            AuthorizationDecision::Deny(AuthorizationDenyReason::InvalidPolicyTable)
        );
        assert_eq!(
            authorize_with_decision(
                &Principal {
                    subject: "alice".to_string(),
                    roles: vec!["analyst\nadmin".to_string()],
                },
                &policy
            ),
            AuthorizationDecision::Deny(AuthorizationDenyReason::InvalidRole)
        );
    }

    #[test]
    fn denies_empty_required_role_and_missing_role_with_exact_reason() {
        let base = AuthPolicy {
            table: "analytics.gold_daily_returns".to_string(),
            required_role: "analyst".to_string(),
            row_filter: None,
        };

        // Policy with a blank required_role.
        assert_eq!(
            authorize_with_decision(
                &Principal {
                    subject: "alice".to_string(),
                    roles: vec!["analyst".to_string()],
                },
                &AuthPolicy {
                    required_role: "  ".to_string(),
                    ..base.clone()
                },
            ),
            AuthorizationDecision::Deny(AuthorizationDenyReason::EmptyRequiredRole)
        );

        // Valid subject/table/roles, but the principal lacks the required role.
        assert_eq!(
            authorize_with_decision(
                &Principal {
                    subject: "alice".to_string(),
                    roles: vec!["guest".to_string()],
                },
                &base,
            ),
            AuthorizationDecision::Deny(AuthorizationDenyReason::MissingRequiredRole)
        );
    }
}
