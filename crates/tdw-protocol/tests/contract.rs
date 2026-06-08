#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used)]

// Drop-in integration tests for tdw-protocol — every Op + EventMsg variant
// round-trips through serde_json, plus ID newtype edge cases.
//
// Drop this file at: crates/tdw-protocol/tests/contract.rs
// IMPORTANT: Authored offline. Verify with
// `cargo test --package tdw-protocol` before merging.

use serde_json::{Value, json};
use tdw_protocol::{
    ActorKind, ActorRef, ApprovalDecision, CostHint, EventMsg, Op, OpEnvelope, OpId, OutputStream,
    PermissionId, PlanId, ProtocolError, ReplayFrame, SessionId, TimeRange, ToolCallId,
    schema_bundle,
};

fn actor() -> ActorRef {
    ActorRef {
        actor_id: "user:alice".to_string(),
        kind: ActorKind::User,
        tenant_id: Some("default".to_string()),
    }
}

fn round_trip<T>(value: &T) -> T
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug + Clone,
{
    let encoded = serde_json::to_value(value).unwrap_or_else(|error| panic!("serialize: {error}"));
    let decoded: T =
        serde_json::from_value(encoded).unwrap_or_else(|error| panic!("deserialize: {error}"));
    assert_eq!(&decoded, value);
    decoded
}

// ----- ID newtypes -----

#[test]
fn session_id_rejects_empty_and_whitespace() {
    assert!(matches!(
        SessionId::new(""),
        Err(ProtocolError::EmptyField {
            field: "session_id"
        })
    ));
    assert!(matches!(
        SessionId::new("   "),
        Err(ProtocolError::EmptyField {
            field: "session_id"
        })
    ));
}

#[test]
fn session_id_generated_is_non_empty_and_prefixed() {
    let id = SessionId::generated();
    assert!(id.as_str().starts_with("session-"));
    assert!(id.as_str().len() > "session-".len());
}

#[test]
fn permission_id_rejects_empty() {
    assert!(matches!(
        PermissionId::new(""),
        Err(ProtocolError::EmptyField {
            field: "permission_id"
        })
    ));
}

#[test]
fn plan_id_rejects_empty() {
    assert!(matches!(
        PlanId::new(""),
        Err(ProtocolError::EmptyField { field: "plan_id" })
    ));
}

#[test]
fn tool_call_id_rejects_empty() {
    assert!(matches!(
        ToolCallId::new(""),
        Err(ProtocolError::EmptyField {
            field: "tool_call_id"
        })
    ));
}

#[test]
fn op_id_generated_uses_uuid_format() {
    let id = OpId::generated();
    let s = id.as_str();
    // UUIDv7 is 36 chars with 4 hyphens
    assert_eq!(s.len(), 36);
    assert_eq!(s.matches('-').count(), 4);
}

// ----- Op variants -----

#[test]
fn op_append_user_message_round_trips() {
    let op = Op::AppendUserMessage {
        message: "hello".to_string(),
    };
    round_trip(&op);
}

#[test]
fn op_run_query_round_trips_without_optionals() {
    let op = Op::RunQuery {
        sql: "select 1".to_string(),
        plan_id: None,
        cost_hint: None,
    };
    round_trip(&op);
}

#[test]
fn op_run_query_round_trips_with_all_fields() {
    let op = Op::RunQuery {
        sql: "select * from t".to_string(),
        plan_id: Some(PlanId::new("plan-1").unwrap_or_else(|e| panic!("plan id: {e}"))),
        cost_hint: Some(CostHint {
            backend: "clickhouse".to_string(),
            estimated_bytes_scanned: Some(1024),
            estimated_rows_read: Some(10),
        }),
    };
    round_trip(&op);
}

#[test]
fn op_ingest_batch_round_trips_with_time_range() {
    let op = Op::IngestBatch {
        provider: "yahoo".to_string(),
        endpoint: "equity_historical".to_string(),
        symbols: vec!["AAPL".to_string(), "MSFT".to_string()],
        range: Some(TimeRange {
            start: "2026-01-01".to_string(),
            end: "2026-05-22".to_string(),
        }),
    };
    round_trip(&op);
}

#[test]
fn op_tool_call_round_trips_with_permission() {
    let op = Op::ToolCall {
        call_id: ToolCallId::new("call-1").unwrap_or_else(|e| panic!("call id: {e}")),
        tool_name: "tdw.query.run".to_string(),
        arguments: json!({"sql": "select 1"}),
        permission_id: Some(PermissionId::new("perm-1").unwrap_or_else(|e| panic!("perm id: {e}"))),
    };
    round_trip(&op);
}

#[test]
fn op_approval_response_round_trips_each_decision() {
    for decision in [
        ApprovalDecision::AllowOnce,
        ApprovalDecision::AlwaysAllow,
        ApprovalDecision::Deny,
    ] {
        let op = Op::ApprovalResponse {
            permission_id: PermissionId::new("perm-1").unwrap_or_else(|e| panic!("perm id: {e}")),
            decision,
            reason: Some("policy".to_string()),
        };
        round_trip(&op);
    }
}

#[test]
fn op_compact_context_round_trips() {
    let op = Op::CompactContext {
        target_tokens: 4096,
    };
    round_trip(&op);
}

#[test]
fn op_cancel_round_trips() {
    let op = Op::Cancel {
        op_id: OpId::generated(),
    };
    round_trip(&op);
}

#[test]
fn op_shutdown_round_trips() {
    let op = Op::Shutdown;
    round_trip(&op);
}

#[test]
fn op_get_quote_snapshot_round_trips_and_serialises_as_snake_case() {
    let op = Op::GetQuoteSnapshot {
        provider: "fmp".to_string(),
        symbol: "AAPL".to_string(),
    };
    let encoded = serde_json::to_value(&op).unwrap_or_else(|e| panic!("serialize: {e}"));
    assert_eq!(encoded["type"], "get_quote_snapshot");
    assert_eq!(encoded["provider"], "fmp");
    assert_eq!(encoded["symbol"], "AAPL");
    round_trip(&op);
}

#[test]
fn op_create_alert_round_trips_and_serialises_as_snake_case() {
    // target_price is a decimal String in the Op variant (not f64) so that Op
    // remains Eq-derivable and NaN cannot enter the protocol layer. The
    // dispatcher parses it to f64 and rejects non-finite values.
    let op = Op::CreateAlert {
        symbol: "AAPL".to_string(),
        target_price: "200.00".to_string(),
        condition: "Above".to_string(),
    };
    let encoded = serde_json::to_value(&op).unwrap_or_else(|e| panic!("serialize: {e}"));
    assert_eq!(encoded["type"], "create_alert");
    assert_eq!(encoded["symbol"], "AAPL");
    assert_eq!(encoded["target_price"], "200.00");
    assert_eq!(encoded["condition"], "Above");
    round_trip(&op);
}

#[test]
fn op_create_alert_below_condition_round_trips() {
    let op = Op::CreateAlert {
        symbol: "BTC".to_string(),
        target_price: "50000.00".to_string(),
        condition: "Below".to_string(),
    };
    let encoded = serde_json::to_value(&op).unwrap_or_else(|e| panic!("serialize: {e}"));
    assert_eq!(encoded["condition"], "Below");
    // target_price travels as a JSON string on the wire
    assert!(encoded["target_price"].is_string());
    round_trip(&op);
}

#[test]
fn op_list_alerts_round_trips_and_serialises_as_snake_case() {
    let op = Op::ListAlerts {};
    let encoded = serde_json::to_value(&op).unwrap_or_else(|e| panic!("serialize: {e}"));
    assert_eq!(encoded["type"], "list_alerts");
    round_trip(&op);
}

#[test]
fn op_delete_alert_round_trips_and_serialises_as_snake_case() {
    let op = Op::DeleteAlert {
        id: "01J0AAAAAAAAAAAAAAAAAAAAA1".to_string(),
    };
    let encoded = serde_json::to_value(&op).unwrap_or_else(|e| panic!("serialize: {e}"));
    assert_eq!(encoded["type"], "delete_alert");
    assert_eq!(encoded["id"], "01J0AAAAAAAAAAAAAAAAAAAAA1");
    round_trip(&op);
}

#[test]
fn op_set_alert_active_round_trips_and_serialises_as_snake_case() {
    for active in [true, false] {
        let op = Op::SetAlertActive {
            id: "01J0AAAAAAAAAAAAAAAAAAAAA2".to_string(),
            active,
        };
        let encoded = serde_json::to_value(&op).unwrap_or_else(|e| panic!("serialize: {e}"));
        assert_eq!(encoded["type"], "set_alert_active");
        assert_eq!(encoded["id"], "01J0AAAAAAAAAAAAAAAAAAAAA2");
        assert_eq!(encoded["active"], active);
        round_trip(&op);
    }
}

#[test]
fn op_create_alert_target_price_is_string_on_wire() {
    // Decimal string encoding: target_price is String so Op remains Eq-derivable
    // and NaN cannot enter the protocol. The dispatcher parses to f64 and
    // rejects non-finite / negative values.
    let op = Op::CreateAlert {
        symbol: "ETH".to_string(),
        target_price: "1234.56".to_string(),
        condition: "Above".to_string(),
    };
    let encoded = serde_json::to_value(&op).unwrap_or_else(|e| panic!("serialize: {e}"));
    assert!(
        encoded["target_price"].is_string(),
        "target_price must be a JSON string on the wire, got: {}",
        encoded["target_price"]
    );
    assert_eq!(encoded["target_price"], "1234.56");
    round_trip(&op);
}

// ----- EventMsg variants -----

#[test]
fn event_msg_started_round_trips() {
    let event = EventMsg::Started {
        op_id: OpId::generated(),
    };
    round_trip(&event);
}

#[test]
fn event_msg_progress_round_trips() {
    let event = EventMsg::Progress {
        op_id: OpId::generated(),
        stage: "fetch".to_string(),
        fraction: 0.5,
        message: Some("halfway".to_string()),
    };
    round_trip(&event);
}

#[test]
fn event_msg_approval_requested_round_trips() {
    let event = EventMsg::ApprovalRequested {
        permission_id: PermissionId::new("perm-1").unwrap_or_else(|e| panic!("perm id: {e}")),
        action: "tdw.udf.run".to_string(),
        pattern: "tdw.udf.*".to_string(),
    };
    round_trip(&event);
}

#[test]
fn event_msg_tool_call_requested_round_trips() {
    let event = EventMsg::ToolCallRequested {
        op_id: OpId::generated(),
        call_id: ToolCallId::new("call-1").unwrap_or_else(|e| panic!("call id: {e}")),
        tool_name: "tdw.echo".to_string(),
        arguments: json!({"symbol": "AAPL"}),
    };
    round_trip(&event);
}

#[test]
fn event_msg_tool_call_completed_round_trips() {
    let event = EventMsg::ToolCallCompleted {
        op_id: OpId::generated(),
        call_id: ToolCallId::new("call-1").unwrap_or_else(|e| panic!("call id: {e}")),
        result: json!({"ok": true}),
    };
    round_trip(&event);
}

#[test]
fn event_msg_output_chunk_round_trips_for_each_stream() {
    for stream in [
        OutputStream::Stdout,
        OutputStream::Stderr,
        OutputStream::Model,
        OutputStream::System,
    ] {
        let event = EventMsg::OutputChunk {
            op_id: OpId::generated(),
            stream,
            bytes: "hello".to_string(),
        };
        round_trip(&event);
    }
}

#[test]
fn event_msg_domain_event_round_trips() {
    let event = EventMsg::DomainEvent {
        op_id: OpId::generated(),
        event_type: "query.planned".to_string(),
        payload: json!({"backend": "clickhouse"}),
    };
    round_trip(&event);
}

#[test]
fn event_msg_completed_round_trips_with_result() {
    let event = EventMsg::Completed {
        op_id: OpId::generated(),
        summary: Some("done".to_string()),
        result: Some(json!({"rows": 10})),
    };
    round_trip(&event);
}

#[test]
fn event_msg_failed_round_trips() {
    let event = EventMsg::Failed {
        op_id: OpId::generated(),
        error: "timeout".to_string(),
    };
    round_trip(&event);
}

#[test]
fn event_msg_cancelled_round_trips() {
    let event = EventMsg::Cancelled {
        op_id: OpId::generated(),
        reason: Some("user".to_string()),
    };
    round_trip(&event);
}

// ----- OpEnvelope and ReplayFrame -----

#[test]
fn op_envelope_assigns_fresh_op_id_per_construction() {
    let session = SessionId::new("s-1").unwrap_or_else(|e| panic!("session id: {e}"));
    let env1 = OpEnvelope::new(session.clone(), 1, actor(), Op::Shutdown);
    let env2 = OpEnvelope::new(session, 2, actor(), Op::Shutdown);
    assert_ne!(env1.op_id.as_str(), env2.op_id.as_str());
}

#[test]
fn op_envelope_round_trips_with_tagged_op() {
    let session = SessionId::new("s-1").unwrap_or_else(|e| panic!("session id: {e}"));
    let envelope = OpEnvelope::new(
        session,
        42,
        actor(),
        Op::AppendUserMessage {
            message: "hi".to_string(),
        },
    );
    let encoded =
        serde_json::to_value(&envelope).unwrap_or_else(|error| panic!("serialize: {error}"));
    assert_eq!(encoded["op"]["type"], "append_user_message");
    let decoded: OpEnvelope =
        serde_json::from_value(encoded).unwrap_or_else(|error| panic!("deserialize: {error}"));
    assert_eq!(decoded.sequence, 42);
}

#[test]
fn replay_frame_round_trips_for_each_event_variant() {
    let session = SessionId::new("s-1").unwrap_or_else(|e| panic!("session id: {e}"));
    for (idx, event) in [
        EventMsg::Started {
            op_id: OpId::generated(),
        },
        EventMsg::Failed {
            op_id: OpId::generated(),
            error: "x".to_string(),
        },
        EventMsg::Cancelled {
            op_id: OpId::generated(),
            reason: None,
        },
    ]
    .into_iter()
    .enumerate()
    {
        let frame = ReplayFrame {
            session_id: session.clone(),
            sequence: idx as u64,
            event,
        };
        let encoded = serde_json::to_string(&frame).unwrap_or_else(|e| panic!("serialize: {e}"));
        let decoded: ReplayFrame =
            serde_json::from_str(&encoded).unwrap_or_else(|e| panic!("deserialize: {e}"));
        assert_eq!(decoded, frame);
    }
}

#[test]
fn schema_bundle_contains_all_four_known_keys() {
    let bundle = schema_bundle();
    for name in ["op", "event_msg", "op_envelope", "replay_frame"] {
        let schema: &Value = bundle
            .get(name)
            .unwrap_or_else(|| panic!("missing schema: {name}"));
        assert!(schema.is_object(), "schema {name} should be a JSON object");
    }
}
