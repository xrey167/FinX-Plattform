//! Function-job enqueue port (runtime-wiring slice R3).
//!
//! This module defines the [`FunctionEnqueuer`] trait — the seam the
//! dispatcher calls to fan out subscribed application-function jobs when a
//! domain event is emitted.  The trait has **no production implementation in
//! this crate**: `tdw-worker` has an unconditional path-dependency back onto
//! `tdw-service-api` (`tdw-worker/Cargo.toml:42`), so pulling `tdw-functions`
//! or `tdw-worker` into `tdw-service-api` would form a Cargo manifest-level
//! cycle that cannot be resolved via feature gating.
//!
//! # Production impl deferred
//!
//! `RouterEnqueuer` (which needs `tdw-functions` + `tdw-worker`) cannot live
//! in this crate: `tdw-worker → tdw-service-api` is an unconditional path dep
//! (`tdw-worker/Cargo.toml:42`), creating a Cargo manifest-level cycle.  The
//! concrete production impl is a follow-up in a cycle-free wiring crate once
//! that edge is resolved.

use serde_json::Value;

/// Port the dispatcher calls to enqueue function jobs for an emitted event.
///
/// Implementations enqueue one durable worker job per function subscribed to
/// `event_type`.  The method is synchronous (matches
/// `tdw_worker::DurableWorkerQueue` and
/// `tdw_functions::event_wiring::EventRouter::dispatch`, both sync).
///
/// The field on [`crate::AppState`] is `Option<Arc<dyn FunctionEnqueuer>>`
/// and defaults to `None`, so the feature is strictly off by default and has
/// zero production impact unless explicitly wired.
// The type name shares a prefix with the module name; suppress the pedantic lint.
#[allow(clippy::module_name_repetitions)]
pub trait FunctionEnqueuer: Send + Sync {
    /// Enqueue the function jobs subscribed to `event_type`.
    ///
    /// Returns the number of jobs enqueued.  On error, returns a `String`
    /// message; callers treat this as non-fatal (warn-and-continue).
    ///
    /// # Errors
    ///
    /// Returns `Err(String)` when an enqueue fails.
    fn enqueue_for_event(
        &self,
        event_type: &str,
        payload: Value,
        run_id_seed: &str,
        now_ms: i64,
    ) -> Result<usize, String>;
}

#[cfg(all(test, feature = "functions"))]
mod tests {
    use std::sync::Mutex;

    use serde_json::{Value, json};

    use super::FunctionEnqueuer;

    #[derive(Default)]
    struct RecordingEnqueuer {
        events: Mutex<Vec<String>>,
    }

    impl FunctionEnqueuer for RecordingEnqueuer {
        fn enqueue_for_event(
            &self,
            event_type: &str,
            _payload: Value,
            _run_id_seed: &str,
            _now_ms: i64,
        ) -> Result<usize, String> {
            self.events
                .lock()
                .expect("lock")
                .push(event_type.to_string());
            Ok(1)
        }
    }

    #[test]
    fn recording_enqueuer_implements_trait() {
        let enc = RecordingEnqueuer::default();
        let result = enc.enqueue_for_event("test.event", json!({}), "seed", 0);
        assert_eq!(result, Ok(1));
        let events = enc.events.lock().expect("lock");
        assert_eq!(events.as_slice(), ["test.event"]);
    }
}
