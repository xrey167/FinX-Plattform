//! End-to-end integration test: real ClickHouse OLAP backend.
//!
//! Drives the daemon ingest path against a LIVE ClickHouse:
//!   AppState (ClickHouseHttpEngine olap) -> Dispatcher::dispatch(IngestBatch)
//!   -> dispatch_ingest fetches the provider batch and persists it via
//!      olap.execute(INSERT ... FORMAT JSONEachRow) -> rows land in
//!      raw.equity_historical on the real server -> verified by a follow-up query.
//!
//! # Gating
//! - Feature gate: compiled only with `--features real-clickhouse` (activates
//!   `tdw-storage-clickhouse/clickhouse` and exposes `with_real_clickhouse`).
//! - Runtime gate: reads `TDW_CLICKHOUSE_TEST_URL`; if unset the test returns
//!   immediately (exit 0) so offline CI still passes.
//!
//! # Running locally
//! ```sh
//! # ClickHouse up with the migrations applied (raw.equity_historical exists)
//! export TDW_CLICKHOUSE_TEST_URL="http://127.0.0.1:8123"
//! cargo test -p tdw-service-api --features real-clickhouse \
//!   --test clickhouse_ingest_e2e -- --nocapture
//! ```
#![cfg(feature = "real-clickhouse")]

use serde_json::Value;
use tdw_app_server::Dispatcher;
use tdw_auth_oidc::{JwksKey, JwtClaims};
use tdw_protocol::{ActorKind, ActorRef, EventMsg, Op, OpEnvelope, SessionId};
use tdw_service_api::{AppState, IngressAuthContext, PolicyEnforcementConfig};

fn analyst_policy() -> PolicyEnforcementConfig {
    PolicyEnforcementConfig {
        auth: IngressAuthContext {
            claims: JwtClaims {
                sub: "alice".to_string(),
                iss: "https://issuer".to_string(),
                aud: "tdw".to_string(),
                kid: "k1".to_string(),
                roles: vec!["analyst".to_string()],
            },
            jwks: vec![JwksKey {
                kid: "k1".to_string(),
                alg: "RS256".to_string(),
            }],
            issuer: "https://issuer".to_string(),
            audience: "tdw".to_string(),
        },
        hooks: Vec::new(),
        hook_execution: Default::default(),
        mask_rules: Vec::new(),
    }
}

/// Parse a ClickHouse `FORMAT JSON` scalar that may arrive as a JSON string
/// (UInt64 are serialized as strings) or a number.
fn as_count(v: &Value) -> u64 {
    v.as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| v.as_u64())
        .unwrap_or(0)
}

#[tokio::test]
async fn e2e_real_clickhouse_ingest_persists() {
    let Ok(url) = std::env::var("TDW_CLICKHOUSE_TEST_URL") else {
        eprintln!("skipping CH e2e: TDW_CLICKHOUSE_TEST_URL not set");
        return;
    };
    let user = std::env::var("TDW_CLICKHOUSE_USER").ok();
    let password = std::env::var("TDW_CLICKHOUSE_PASSWORD").ok();

    // Build AppState with in-memory stores but a REAL ClickHouse OLAP engine.
    let state = AppState::in_memory_for_tests()
        .await
        .with_policy(analyst_policy())
        .with_real_clickhouse(&url, user, password)
        .expect("ClickHouseHttpEngine build failed — check TDW_CLICKHOUSE_TEST_URL");

    // Drive an ingest op through the dispatcher; dispatch_ingest persists the
    // fetched yahoo/equity_historical batch (symbol AAPL) into raw.equity_historical.
    let env = OpEnvelope::new(
        SessionId::new("session-e2e-ch").expect("session id"),
        1,
        ActorRef {
            actor_id: "user:test-e2e".to_string(),
            kind: ActorKind::User,
            tenant_id: Some("default".to_string()),
        },
        Op::IngestBatch {
            provider: "yahoo".to_string(),
            endpoint: "equity_historical".to_string(),
            range: None,
        },
    );

    let events = state.dispatch(env).await;
    let result = events
        .iter()
        .find_map(|e| match e {
            EventMsg::Completed {
                result: Some(v), ..
            } => Some(v.clone()),
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "ingest did not Complete (is ClickHouse reachable and raw.equity_historical migrated?): {events:?}"
            )
        });

    assert_eq!(result["table"], "raw.equity_historical");
    assert!(
        result["rows"].as_u64().unwrap_or(0) >= 1,
        "expected >=1 persisted row, got {result}"
    );

    // Verify the row actually landed in the live server.
    let q = state
        .olap
        .query_json(
            "SELECT count() AS c FROM raw.equity_historical WHERE symbol = 'AAPL'",
            Value::Null,
        )
        .await
        .expect("query against live ClickHouse failed");
    let c = as_count(&q["data"][0]["c"]);
    assert!(
        c >= 1,
        "expected >=1 AAPL row in live ClickHouse after ingest, got {c}; response={q}"
    );
}
