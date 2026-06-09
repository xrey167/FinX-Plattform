//! Offline `tdw-acp` example: validate ACP requests at the boundary, and parse
//! an approval decision.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p tdw-acp --example tdw_acp_basic
//! ```

#![forbid(unsafe_code)]

use tdw_acp::{AcpRequest, AcpValidationError, parse_approval_decision, validate_request};
use tdw_protocol::{ApprovalDecision, Op, SessionId};

fn main() {
    // A well-formed initialize request passes.
    let init = AcpRequest::Initialize {
        client_name: "tdw-cli".to_string(),
    };
    assert!(validate_request(&init).is_ok());
    println!("initialize accepted");

    // A read-only SubmitOp passes.
    let good_query = AcpRequest::SubmitOp {
        session_id: SessionId::new("session-1").expect("session id"),
        op: Op::RunQuery {
            sql: "select 1".to_string(),
            plan_id: None,
            cost_hint: None,
        },
    };
    assert!(validate_request(&good_query).is_ok());
    println!("read-only SubmitOp accepted");

    // Statement stacking in a SubmitOp query is rejected.
    let stacked = AcpRequest::SubmitOp {
        session_id: SessionId::new("session-1").expect("session id"),
        op: Op::RunQuery {
            sql: "select 1; drop table raw.orders".to_string(),
            plan_id: None,
            cost_hint: None,
        },
    };
    assert_eq!(
        validate_request(&stacked),
        Err(AcpValidationError::InvalidQuery {
            reason: "multiple statements are not allowed",
        }),
    );
    println!("multi-statement query rejected");

    // Path traversal in a permission id is an unsafe field.
    let traversal = AcpRequest::ResolveApproval {
        permission_id: "../approval".to_string(),
        decision: "allow_once".to_string(),
    };
    assert_eq!(
        validate_request(&traversal),
        Err(AcpValidationError::UnsafeField {
            field: "permission_id",
        }),
    );
    println!("permission-id traversal rejected");

    // Approval-decision parsing is case- and separator-insensitive.
    assert_eq!(
        parse_approval_decision("Allow-Once"),
        Ok(ApprovalDecision::AllowOnce),
    );
    println!(
        "parsed approval decision: {:?}",
        ApprovalDecision::AllowOnce
    );
}
