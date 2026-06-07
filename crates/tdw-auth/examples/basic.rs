//! Offline `tdw-auth` example: authorize a principal against a table policy,
//! and show the fail-closed deny reasons.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p tdw-auth --example tdw_auth_basic
//! ```

#![forbid(unsafe_code)]

use tdw_auth::{
    AuthPolicy, AuthorizationDecision, AuthorizationDenyReason, Principal, authorize,
    authorize_with_decision,
};

fn main() {
    let policy = AuthPolicy {
        table: "analytics.gold_daily_returns".to_string(),
        required_role: "analyst".to_string(),
        row_filter: Some("tenant_id = current_tenant()".to_string()),
    };

    // Holds the required role -> allowed.
    let alice = Principal {
        subject: "alice".to_string(),
        roles: vec!["analyst".to_string()],
    };
    assert!(authorize(&alice, &policy));
    println!("alice (analyst) -> allowed");

    // Missing the role -> denied with a reason.
    let bob = Principal {
        subject: "bob".to_string(),
        roles: vec!["guest".to_string()],
    };
    assert_eq!(
        authorize_with_decision(&bob, &policy),
        AuthorizationDecision::Deny(AuthorizationDenyReason::MissingRequiredRole),
    );
    println!("bob (guest) -> denied: missing required role");

    // Fail-closed: an unsafe table path is rejected before any role check.
    let unsafe_policy = AuthPolicy {
        table: "analytics.gold_daily_returns;drop".to_string(),
        ..policy
    };
    assert_eq!(
        authorize_with_decision(&alice, &unsafe_policy),
        AuthorizationDecision::Deny(AuthorizationDenyReason::InvalidPolicyTable),
    );
    println!("unsafe table path -> denied: invalid policy table");
}
