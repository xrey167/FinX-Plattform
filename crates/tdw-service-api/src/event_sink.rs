//! `EventSink` implementation for `AppState` (P2 of the integration cycle).
//!
//! Wires each dispatched `EventMsg` into three durable stores:
//! - `InMemoryOutbox` for downstream relay (bus / CDC)
//! - `JsonlRollout` for replay / audit
//! - `SqliteSessionStore` cost ledger for per-operation accounting

use async_trait::async_trait;
use serde_json::Value;
use tdw_app_server::{EventSink, SinkError, SinkResult};
use tdw_event::{EventEnvelope, sample_actor_context};
use tdw_protocol::{EventMsg, OpEnvelope, ReplayFrame};
use tdw_rollout::RolloutRecord;
use tdw_session::{CostLedgerEntry, SessionRecord, SessionStatus};

use crate::AppState;

#[async_trait]
impl EventSink for AppState {
    async fn persist_event(
        &self,
        env: &OpEnvelope,
        event: &EventMsg,
        sequence: u64,
    ) -> SinkResult<()> {
        // 1. Append to the in-memory outbox.
        let payload =
            serde_json::to_value(event).map_err(|e| SinkError(format!("serialize event: {e}")))?;
        let (actor, origin, trace) = sample_actor_context("tdw-service");
        let envelope: EventEnvelope<Value> = EventEnvelope::new(
            "daemon.event",
            actor,
            origin,
            trace,
            "2026-05-28T00:00:00Z",
            payload,
        );
        {
            let mut outbox = self
                .outbox
                .lock()
                .unwrap_or_else(|e| panic!("outbox lock poisoned: {e}"));
            outbox.append(envelope);
        }

        // 2. Append to the JSONL rollout file.
        let record = RolloutRecord {
            recorded_at: "2026-05-28T00:00:00Z".to_string(),
            frame: ReplayFrame {
                session_id: env.session_id.clone(),
                sequence,
                event: event.clone(),
            },
        };
        self.rollout
            .append(&record)
            .await
            .map_err(|e| SinkError(format!("rollout append: {e}")))?;

        Ok(())
    }

    async fn record_cost(&self, env: &OpEnvelope, backend: &str) -> SinkResult<()> {
        // Ensure a session row exists (foreign-key constraint).
        let session_record = SessionRecord {
            session_id: env.session_id.as_str().to_string(),
            status: SessionStatus::Active,
            created_at: "2026-05-28T00:00:00Z".to_string(),
            updated_at: "2026-05-28T00:00:00Z".to_string(),
        };
        self.session
            .upsert_session(&session_record)
            .await
            .map_err(|e| SinkError(format!("upsert session: {e}")))?;

        let entry = CostLedgerEntry {
            session_id: env.session_id.as_str().to_string(),
            operation_id: env.op_id.as_str().to_string(),
            tokens: 0,
            bytes_scanned: 0,
            rows_read: 0,
            rows_written: 0,
            backend: backend.to_string(),
        };
        self.session
            .append_cost(&entry)
            .await
            .map_err(|e| SinkError(format!("append cost: {e}")))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_app_server::service_channel;
    use tdw_auth_oidc::{JwksKey, JwtClaims};
    use tdw_protocol::{ActorKind, ActorRef, Op, SessionId};

    use crate::{AppState, IngressAuthContext, PolicyEnforcementConfig};

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
            hook_execution: tdw_hooks::HookExecutionPolicy::default(),
            mask_rules: Vec::new(),
        }
    }

    fn make_envelope(session_id: SessionId, op: Op) -> OpEnvelope {
        OpEnvelope::new(
            session_id,
            1,
            ActorRef {
                actor_id: "user:test".to_string(),
                kind: ActorKind::User,
                tenant_id: Some("default".to_string()),
            },
            op,
        )
    }

    #[tokio::test]
    async fn service_loop_dispatches_persists_and_records_cost() {
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());

        let session_id = SessionId::new("session-sink-test").expect("session id");
        let env = make_envelope(
            session_id.clone(),
            Op::RunQuery {
                sql: "select 1".to_string(),
                plan_id: None,
                cost_hint: None,
            },
        );

        let (handle, _event_rx, mut service_loop) = service_channel(state.clone(), state.clone());

        handle.submit(env.clone()).expect("submission accepted");

        let events = service_loop
            .run_once()
            .await
            .expect("service loop produced events");

        // Should have Started + Completed (or Failed — dispatch succeeds with analyst policy).
        assert_eq!(events.len(), 2, "expected Started + terminal event");
        assert!(
            matches!(&events[0], EventMsg::Started { .. }),
            "first event must be Started"
        );
        assert!(
            matches!(&events[1], EventMsg::Completed { .. }),
            "second event must be Completed, got: {:?}",
            &events[1]
        );

        // Outbox should have 2 entries (one per event).
        let outbox_len = state
            .outbox
            .lock()
            .unwrap_or_else(|e| panic!("outbox lock: {e}"))
            .pending_after(0)
            .len();
        assert_eq!(outbox_len, 2, "outbox must have 2 pending entries");

        // Cost ledger should have exactly 1 entry for this operation.
        let cost_entries = state
            .session
            .cost_entries(&session_id)
            .await
            .unwrap_or_else(|e| panic!("cost_entries: {e}"));
        assert_eq!(cost_entries.len(), 1, "cost ledger must have 1 entry");
        assert_eq!(cost_entries[0].backend, "in-memory");

        // Rollout file should have 2 records.
        let rollout_records = state
            .rollout
            .read_all()
            .await
            .unwrap_or_else(|e| panic!("rollout read_all: {e}"));
        assert_eq!(rollout_records.len(), 2, "rollout must have 2 records");
    }
}
