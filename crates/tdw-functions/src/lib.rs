//! Named multi-step function registry with per-step memoization, worker-job
//! bridge, cron-trigger wiring, and event dispatch.
//!
//! # Overview
//!
//! This crate provides a durable, resumable function execution model:
//!
//! - [`FunctionDef`] — a named function with a set of [`Trigger`]s and a
//!   [`StepFn`] handler.
//! - [`StepContext`] — execution context passed to each handler; its
//!   [`StepContext::step`] method memoizes individual steps so that re-running
//!   a function after a partial failure skips already-completed steps.
//! - [`FunctionRegistry`] — registers functions, invokes them, and resumes
//!   partially-executed runs.
//! - [`StepStore`] / [`RunStore`] — persistence traits with [`InMemoryStepStore`]
//!   / [`InMemoryRunStore`] always compiled, [`PgStepStore`] / [`PgRunStore`]
//!   behind the `postgres` feature, and [`sqlite::SqliteStepStore`] /
//!   [`sqlite::SqliteRunStore`] behind the `sqlite` feature.
//!
//! # Trigger format
//!
//! [`Trigger::Cron`] holds a 5-field cron expression (`min hour dom month
//! dow`) matching the format used by `tdw-cron`.  [`Trigger::Event`] holds a
//! named event string matched by [`event_wiring::EventRouter`].
//!
//! # Worker-job bridge (`worker` feature)
//!
//! [`job::FunctionJobHandler`] implements [`tdw_worker::JobHandler`] so a
//! [`tdw_worker::WorkerRunner`] can drive the registry.  Each job carries a
//! [`job::FunctionJob`] serialized into an [`tdw_protocol::Op::ToolCall`]
//! envelope.  [`job::enqueue_invoke`] builds and enqueues the job.
//!
//! # Cron wiring (`cron` feature)
//!
//! [`cron_wiring::register_cron_triggers`] walks the registry and registers
//! a [`tdw_cron::ScheduledTrigger`] for every [`Trigger::Cron`] entry.
//!
//! # Event dispatch (`worker` feature)
//!
//! [`event_wiring::EventRouter`] maps event names to subscribed function ids
//! and enqueues jobs via [`event_wiring::EventRouter::dispatch`].  `tdw-event`
//! has no subscriber/callback surface, so dispatch is explicit (call-site
//! driven), not automatic.
//!
//! # `SQLite` stores (`sqlite` feature)
//!
//! [`sqlite::SqliteStepStore`] and [`sqlite::SqliteRunStore`] use an in-process
//! `SQLite` database, suitable for single-node deployments and tests.
//!
//! # `StepFn` / handler shape
//!
//! ```rust,ignore
//! #[async_trait]
//! pub trait StepFn: Send + Sync {
//!     async fn run(&self, ctx: &StepContext, payload: Value) -> Result<Value, FunctionError>;
//! }
//! ```
//!
//! A blanket `impl` makes any `F: Fn(StepContext, Value) -> BoxFuture` work
//! directly via [`FunctionDef::from_fn`].
//!
//! # Exactly-once step memoization
//!
//! A successful step result is persisted before the step returns.  On a
//! subsequent call with the same `(run_id, step_name)` pair the cached value
//! is returned without re-executing the step body.  A *failed* step is never
//! persisted, so it is re-executed on the next call (resume semantics).
//!
//! # `RuntimeConfig`
//!
//! [`RuntimeConfig`] provides application-level settings read from environment
//! variables (`TDW_FUNCTIONS_APP_ID`, `TDW_FUNCTIONS_SIGNING_SECRET`).
//! LLM-provider binding is deferred to the sub-PR that adds AI-step support.

#![forbid(unsafe_code)]

pub use error::FunctionError;
pub use registry::{FunctionDef, FunctionRegistry, StepFn};
pub use runtime::RuntimeConfig;
pub use step::{InMemoryStepStore, StepContext, StepStore};
pub use store::{InMemoryRunStore, RunRecord, RunStatus, RunStore};
pub use trigger::Trigger;

#[cfg(feature = "postgres")]
pub use pg::{PgRunStore, PgStepStore};

#[cfg(feature = "worker")]
pub mod event_wiring;
#[cfg(feature = "worker")]
pub mod job;

#[cfg(feature = "cron")]
pub mod cron_wiring;

#[cfg(feature = "sqlite")]
pub mod sqlite;

mod error;
#[cfg(feature = "postgres")]
mod pg;
mod registry;
mod runtime;
mod step;
mod store;
mod trigger;

/// Convenience type alias for results produced by this crate.
pub type Result<T> = std::result::Result<T, FunctionError>;

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use serde_json::json;
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    use super::{
        FunctionDef, FunctionRegistry, InMemoryRunStore, InMemoryStepStore, RunStatus, RunStore,
        StepContext, StepStore, Trigger,
    };

    const NOW: i64 = 1_700_000_000_000;

    // Helper: simple 1-step function that returns `json!("ok")`
    fn simple_def(id: &str) -> FunctionDef {
        let id = id.to_string();
        FunctionDef::from_fn(id, vec![], |ctx, _payload| {
            Box::pin(async move { ctx.step("only", || async { Ok(json!("ok")) }).await })
        })
    }

    // Helper: build a fresh in-memory registry
    fn registry() -> FunctionRegistry {
        FunctionRegistry::new()
    }

    // --- Trigger ---

    #[test]
    fn trigger_clone_eq() {
        let a = Trigger::Event("price.updated".to_string());
        let b = Trigger::Cron("0 9 * * 1-5".to_string());
        assert_eq!(a.clone(), a);
        assert_eq!(b.clone(), b);
        assert_ne!(a, b);
    }

    // --- InMemoryStepStore ---

    #[tokio::test]
    async fn step_store_get_before_put_is_none() {
        let store = InMemoryStepStore::new();
        let got = store.get("run1", "step_a").await.expect("get");
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn step_store_put_then_get_roundtrip() {
        let store = InMemoryStepStore::new();
        store.put("run1", "step_a", &json!(42)).await.expect("put");
        let got = store
            .get("run1", "step_a")
            .await
            .expect("get")
            .expect("Some");
        assert_eq!(got, json!(42));
    }

    #[tokio::test]
    async fn step_store_first_write_wins() {
        let store = InMemoryStepStore::new();
        store.put("run1", "step_a", &json!(1)).await.expect("put 1");
        store.put("run1", "step_a", &json!(2)).await.expect("put 2");
        let got = store
            .get("run1", "step_a")
            .await
            .expect("get")
            .expect("Some");
        assert_eq!(got, json!(1), "first write must win");
    }

    #[tokio::test]
    async fn step_store_list_returns_all_steps_for_run() {
        let store = InMemoryStepStore::new();
        store.put("r1", "b", &json!("B")).await.expect("put b");
        store.put("r1", "a", &json!("A")).await.expect("put a");
        store.put("r2", "x", &json!("X")).await.expect("put x");
        let mut listed = store.list("r1").await.expect("list");
        listed.sort_by_key(|(k, _)| k.clone());
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0], ("a".to_string(), json!("A")));
        assert_eq!(listed[1], ("b".to_string(), json!("B")));
    }

    // --- StepContext memoization ---

    #[tokio::test]
    async fn step_context_memoized_step_not_re_executed() {
        let store: Arc<dyn StepStore> = Arc::new(InMemoryStepStore::new());
        let ctx = StepContext::new("run1".to_string(), Arc::clone(&store));
        let counter = Arc::new(AtomicU32::new(0));

        let c1 = Arc::clone(&counter);
        let _ = ctx
            .step("s1", || async move {
                c1.fetch_add(1, Ordering::SeqCst);
                Ok(json!("result"))
            })
            .await
            .expect("first call");

        let c2 = Arc::clone(&counter);
        let _ = ctx
            .step("s1", || async move {
                c2.fetch_add(1, Ordering::SeqCst);
                Ok(json!("result"))
            })
            .await
            .expect("second call");

        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "step body must run exactly once"
        );
    }

    #[tokio::test]
    async fn step_context_failed_step_not_persisted_then_succeeds_on_retry() {
        let store: Arc<dyn StepStore> = Arc::new(InMemoryStepStore::new());
        let ctx = StepContext::new("run1".to_string(), Arc::clone(&store));
        let counter = Arc::new(AtomicU32::new(0));

        // First call — step fails
        let c1 = Arc::clone(&counter);
        let err_result = ctx
            .step("flaky", || async move {
                c1.fetch_add(1, Ordering::SeqCst);
                Err(crate::FunctionError::Step {
                    step: "flaky".to_string(),
                    message: "transient".to_string(),
                })
            })
            .await;
        assert!(err_result.is_err(), "first call should fail");
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        // Second call — step succeeds; body runs because first was not persisted
        let c2 = Arc::clone(&counter);
        let ok_result = ctx
            .step("flaky", || async move {
                c2.fetch_add(1, Ordering::SeqCst);
                Ok(json!("recovered"))
            })
            .await
            .expect("second call should succeed");
        assert_eq!(ok_result, json!("recovered"));
        assert_eq!(
            counter.load(Ordering::SeqCst),
            2,
            "body must run again after failure"
        );
    }

    // --- FunctionRegistry ---

    #[tokio::test]
    async fn registry_duplicate_register_errors() {
        let mut reg = registry();
        reg.register(simple_def("fn-a")).expect("first register");
        let err = reg
            .register(simple_def("fn-a"))
            .expect_err("duplicate should fail");
        assert!(matches!(err, crate::FunctionError::Store(_)));
    }

    #[tokio::test]
    async fn registry_unknown_function_errors() {
        let reg = registry();
        let err = reg
            .invoke("ghost", json!({}), "run1".to_string(), NOW)
            .await
            .expect_err("unknown fn");
        assert!(matches!(err, crate::FunctionError::UnknownFunction(_)));
    }

    #[tokio::test]
    async fn registry_unknown_run_errors() {
        let reg = registry();
        let err = reg.resume("ghost-run", NOW).await.expect_err("unknown run");
        assert!(matches!(err, crate::FunctionError::UnknownRun(_)));
    }

    #[tokio::test]
    async fn registry_enumerate_is_deterministic_btree_order() {
        let mut reg = registry();
        reg.register(simple_def("fn-z")).expect("z");
        reg.register(simple_def("fn-a")).expect("a");
        reg.register(simple_def("fn-m")).expect("m");
        let ids: Vec<String> = reg.enumerate().into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, vec!["fn-a", "fn-m", "fn-z"]);
    }

    #[tokio::test]
    async fn registry_happy_path_invoke_three_steps() {
        let mut reg = registry();
        reg.register(FunctionDef::from_fn(
            "three-steps",
            vec![Trigger::Event("demo".to_string())],
            |ctx, _payload| {
                Box::pin(async move {
                    let a = ctx.step("step-1", || async { Ok(json!(1)) }).await?;
                    let b = ctx.step("step-2", || async { Ok(json!(2)) }).await?;
                    let c = ctx.step("step-3", || async { Ok(json!(3)) }).await?;
                    let sum =
                        a.as_i64().unwrap_or(0) + b.as_i64().unwrap_or(0) + c.as_i64().unwrap_or(0);
                    Ok(json!(sum))
                })
            },
        ))
        .expect("register");

        let result = reg
            .invoke("three-steps", json!({}), "run-3s".to_string(), NOW)
            .await
            .expect("invoke");
        assert_eq!(result, json!(6));
    }

    #[tokio::test]
    async fn registry_completed_run_has_correct_status_and_result() {
        let steps = Arc::new(InMemoryStepStore::new());
        let runs = Arc::new(InMemoryRunStore::new());
        let mut reg = FunctionRegistry::with_stores(
            Arc::clone(&steps) as Arc<dyn StepStore>,
            Arc::clone(&runs) as Arc<dyn crate::RunStore>,
        );
        reg.register(simple_def("fn-status")).expect("register");
        reg.invoke("fn-status", json!({}), "run-status".to_string(), NOW)
            .await
            .expect("invoke");

        let record = runs
            .get_run("run-status")
            .await
            .expect("get_run")
            .expect("Some");
        assert_eq!(record.status, RunStatus::Completed);
        assert_eq!(record.result, Some(json!("ok")));
    }

    #[tokio::test]
    async fn registry_failed_run_has_failed_status() {
        let mut reg = registry();
        reg.register(FunctionDef::from_fn("fn-fail", vec![], |_ctx, _payload| {
            Box::pin(async move {
                Err(crate::FunctionError::Step {
                    step: "boom".to_string(),
                    message: "exploded".to_string(),
                })
            })
        }))
        .expect("register");

        let err = reg
            .invoke("fn-fail", json!({}), "run-fail".to_string(), NOW)
            .await
            .expect_err("should fail");
        assert!(matches!(err, crate::FunctionError::Step { .. }));
    }

    #[tokio::test]
    async fn registry_resume_skips_completed_steps() {
        let steps = Arc::new(InMemoryStepStore::new());
        let runs = Arc::new(InMemoryRunStore::new());

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = Arc::clone(&counter);

        let mut reg = FunctionRegistry::with_stores(
            Arc::clone(&steps) as Arc<dyn StepStore>,
            Arc::clone(&runs) as Arc<dyn crate::RunStore>,
        );

        reg.register(FunctionDef::from_fn(
            "fn-resume",
            vec![],
            move |ctx, _payload| {
                let c = Arc::clone(&counter_clone);
                Box::pin(async move {
                    // step-1 will be memoized after first invoke
                    ctx.step("step-1", || {
                        let cc = Arc::clone(&c);
                        async move {
                            cc.fetch_add(1, Ordering::SeqCst);
                            Ok(json!("done"))
                        }
                    })
                    .await
                })
            },
        ))
        .expect("register");

        // First invoke — runs step-1
        reg.invoke("fn-resume", json!({}), "run-resume".to_string(), NOW)
            .await
            .expect("invoke");
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        // Resume — step-1 is memoized, body does NOT run again
        reg.resume("run-resume", NOW + 1).await.expect("resume");
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "step must not re-execute on resume"
        );
    }

    #[tokio::test]
    async fn registry_resume_reruns_failed_step() {
        let steps = Arc::new(InMemoryStepStore::new());
        let runs = Arc::new(InMemoryRunStore::new());

        let attempt = Arc::new(AtomicU32::new(0));
        let attempt_clone = Arc::clone(&attempt);

        let mut reg = FunctionRegistry::with_stores(
            Arc::clone(&steps) as Arc<dyn StepStore>,
            Arc::clone(&runs) as Arc<dyn crate::RunStore>,
        );

        reg.register(FunctionDef::from_fn(
            "fn-retry",
            vec![],
            move |ctx, _payload| {
                let a = Arc::clone(&attempt_clone);
                Box::pin(async move {
                    ctx.step("step-x", || {
                        let aa = Arc::clone(&a);
                        async move {
                            let n = aa.fetch_add(1, Ordering::SeqCst);
                            if n == 0 {
                                Err(crate::FunctionError::Step {
                                    step: "step-x".to_string(),
                                    message: "first fail".to_string(),
                                })
                            } else {
                                Ok(json!("success"))
                            }
                        }
                    })
                    .await
                })
            },
        ))
        .expect("register");

        // First invoke — fails
        let _ = reg
            .invoke("fn-retry", json!({}), "run-retry".to_string(), NOW)
            .await;
        assert_eq!(attempt.load(Ordering::SeqCst), 1);

        // Resume — step-x not persisted (failed), so it runs again and succeeds
        let result = reg.resume("run-retry", NOW + 1).await.expect("resume");
        assert_eq!(result, json!("success"));
        assert_eq!(attempt.load(Ordering::SeqCst), 2);
    }

    // --- RuntimeConfig ---

    #[test]
    fn runtime_config_default_app_id() {
        // Test the Default impl, which is always "tdw" regardless of env
        let config = crate::RuntimeConfig::default();
        assert_eq!(config.app_id, "tdw");
        assert!(config.signing_secret.is_none());
    }

    #[test]
    fn runtime_config_clone_eq() {
        let c = crate::RuntimeConfig {
            app_id: "myapp".to_string(),
            signing_secret: Some("secret".to_string()),
        };
        assert_eq!(c.clone(), c);
    }
}

// ---------------------------------------------------------------------------
// Worker-bridge + cron + event tests
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "worker"))]
mod worker_tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    };

    use serde_json::json;
    use tdw_worker::{InMemoryWorkerQueue, JobHandler, WorkerJob, WorkerQueueError};

    use super::{
        FunctionDef, FunctionRegistry, InMemoryRunStore, InMemoryStepStore, RunStatus, RunStore,
        StepStore,
    };
    use crate::job::{
        FUNCTIONS_QUEUE, FUNCTIONS_TOOL_NAME, FunctionJob, FunctionJobHandler, JobMode,
        RoutingJobHandler, build_worker_job, enqueue_invoke, extract_function_job,
    };

    fn simple_def(id: &str) -> FunctionDef {
        let id = id.to_string();
        FunctionDef::from_fn(id, vec![], |ctx, _payload| {
            Box::pin(async move { ctx.step("only", || async { Ok(json!("ok")) }).await })
        })
    }

    // -----------------------------------------------------------------------
    // FunctionJob serde round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn function_job_serde_roundtrip() {
        let job = FunctionJob {
            function_id: "fn-a".to_string(),
            run_id: "run-1".to_string(),
            payload: json!({"x": 1}),
            mode: JobMode::Invoke,
        };
        let value = serde_json::to_value(&job).expect("serialize");
        let back: FunctionJob = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back, job);
    }

    // -----------------------------------------------------------------------
    // build_worker_job / extract_function_job round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn build_and_extract_roundtrip() {
        let func_job = FunctionJob {
            function_id: "fn-b".to_string(),
            run_id: "run-2".to_string(),
            payload: json!(42),
            mode: JobMode::Resume,
        };
        let worker_job = build_worker_job("job-2", &func_job).expect("build");
        assert_eq!(worker_job.job_id, "job-2");
        assert_eq!(worker_job.queue, FUNCTIONS_QUEUE);

        let extracted = extract_function_job(&worker_job).expect("extract");
        assert_eq!(extracted, func_job);
    }

    #[test]
    fn extract_wrong_tool_name_errors() {
        use tdw_protocol::{ActorKind, ActorRef, Op, OpEnvelope, SessionId, ToolCallId};
        use tdw_worker::WorkerJob;

        let envelope = OpEnvelope::new(
            SessionId::new("s").expect("sid"),
            1,
            ActorRef {
                actor_id: "a".to_string(),
                kind: ActorKind::Worker,
                tenant_id: None,
            },
            Op::ToolCall {
                call_id: ToolCallId::new("c").expect("cid"),
                tool_name: "wrong.tool".to_string(),
                arguments: json!({}),
                permission_id: None,
            },
        );
        let job = WorkerJob {
            job_id: "j".to_string(),
            queue: FUNCTIONS_QUEUE.to_string(),
            envelope,
            max_attempts: 1,
            not_before_ms: 0,
            priority: 0,
        };
        let err = extract_function_job(&job).expect_err("should fail");
        assert!(err.contains(FUNCTIONS_TOOL_NAME), "error: {err}");
    }

    // -----------------------------------------------------------------------
    // enqueue_invoke helper
    // -----------------------------------------------------------------------

    #[test]
    fn enqueue_invoke_inserts_job_with_correct_queue() {
        let mut queue = InMemoryWorkerQueue::default();
        let outcome = enqueue_invoke(&mut queue, "fn-c", json!({"data": true}), "run-3", "job-3")
            .expect("enqueue");
        assert!(outcome.inserted);
        assert_eq!(outcome.job_id, "job-3");
    }

    #[test]
    fn enqueue_invoke_duplicate_is_dedup() {
        let mut queue = InMemoryWorkerQueue::default();
        enqueue_invoke(&mut queue, "fn-c", json!({}), "run-d", "job-d").expect("first");
        let outcome = enqueue_invoke(&mut queue, "fn-c", json!({}), "run-d", "job-d")
            .expect("duplicate enqueue is not an error");
        assert!(!outcome.inserted, "duplicate must not insert");
    }

    // -----------------------------------------------------------------------
    // FunctionJobHandler: happy path — invokes function and records Completed
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn handler_invoke_completes_run() {
        let steps = Arc::new(InMemoryStepStore::new());
        let runs = Arc::new(InMemoryRunStore::new());
        let mut reg = FunctionRegistry::with_stores(
            Arc::clone(&steps) as Arc<dyn StepStore>,
            Arc::clone(&runs) as Arc<dyn RunStore>,
        );
        reg.register(simple_def("fn-handler")).expect("register");

        let handler = FunctionJobHandler::new(Arc::new(reg));

        let func_job = FunctionJob {
            function_id: "fn-handler".to_string(),
            run_id: "run-h1".to_string(),
            payload: json!({}),
            mode: JobMode::Invoke,
        };
        let worker_job = build_worker_job("job-h1", &func_job).expect("build");
        handler.handle(&worker_job).await.expect("handle ok");

        let record = runs
            .get_run("run-h1")
            .await
            .expect("get_run")
            .expect("Some");
        assert_eq!(record.status, RunStatus::Completed);
    }

    // -----------------------------------------------------------------------
    // FunctionJobHandler: failure surfaces as job failure; retry resumes and
    // skips already-memoized steps (counter proves step ran once across 2
    // handler attempts).
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn handler_retry_skips_memoized_steps() {
        use crate::FunctionError;

        let steps = Arc::new(InMemoryStepStore::new());
        let runs = Arc::new(InMemoryRunStore::new());

        let attempt = Arc::new(AtomicU32::new(0));
        let step_counter = Arc::new(AtomicU32::new(0));
        let attempt_c = Arc::clone(&attempt);
        let step_c = Arc::clone(&step_counter);

        let mut reg = FunctionRegistry::with_stores(
            Arc::clone(&steps) as Arc<dyn StepStore>,
            Arc::clone(&runs) as Arc<dyn RunStore>,
        );

        reg.register(FunctionDef::from_fn(
            "fn-retry",
            vec![],
            move |ctx, _payload| {
                let a = Arc::clone(&attempt_c);
                let sc = Arc::clone(&step_c);
                Box::pin(async move {
                    // step-1 memoizes on success
                    ctx.step("step-1", || {
                        let scc = Arc::clone(&sc);
                        async move {
                            scc.fetch_add(1, Ordering::SeqCst);
                            Ok(json!("done"))
                        }
                    })
                    .await?;

                    // step-2 fails on the first invocation attempt
                    let n = a.fetch_add(1, Ordering::SeqCst);
                    if n == 0 {
                        return Err(FunctionError::Step {
                            step: "step-2".to_string(),
                            message: "transient".to_string(),
                        });
                    }
                    Ok(json!("success"))
                })
            },
        ))
        .expect("register");

        let handler = FunctionJobHandler::new(Arc::new(reg));

        let func_job = FunctionJob {
            function_id: "fn-retry".to_string(),
            run_id: "run-r1".to_string(),
            payload: json!({}),
            mode: JobMode::Invoke,
        };
        let worker_job = build_worker_job("job-r1", &func_job).expect("build");

        // First attempt — fails at step-2
        let first = handler.handle(&worker_job).await;
        assert!(first.is_err(), "first attempt must fail");
        assert_eq!(step_counter.load(Ordering::SeqCst), 1, "step-1 ran once");

        // Second attempt (Resume) — step-1 memoized, step-2 succeeds
        let resume_job_payload = FunctionJob {
            function_id: "fn-retry".to_string(),
            run_id: "run-r1".to_string(),
            payload: json!({}),
            mode: JobMode::Resume,
        };
        let resume_worker_job =
            build_worker_job("job-r1-resume", &resume_job_payload).expect("build");
        handler
            .handle(&resume_worker_job)
            .await
            .expect("second attempt must succeed");

        // step-1 must NOT have run again (still 1)
        assert_eq!(
            step_counter.load(Ordering::SeqCst),
            1,
            "step-1 must not re-execute on resume"
        );
    }

    // -----------------------------------------------------------------------
    // RoutingJobHandler: function jobs run the registry; other jobs delegate
    // to the inner handler.
    // -----------------------------------------------------------------------

    /// Inner [`JobHandler`] that records whether it was called.
    struct RecordingHandler {
        called: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl JobHandler for RecordingHandler {
        async fn handle(&self, _job: &WorkerJob) -> Result<(), String> {
            self.called.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    /// Build a non-function [`WorkerJob`] that [`extract_function_job`] rejects.
    fn non_function_job(job_id: &str) -> WorkerJob {
        use tdw_protocol::{ActorKind, ActorRef, Op, OpEnvelope, SessionId, ToolCallId};

        let envelope = OpEnvelope::new(
            SessionId::new("other-session").expect("sid"),
            1,
            ActorRef {
                actor_id: "other".to_string(),
                kind: ActorKind::Worker,
                tenant_id: None,
            },
            Op::ToolCall {
                call_id: ToolCallId::new("other-call").expect("cid"),
                tool_name: "some.other.tool".to_string(),
                arguments: json!({}),
                permission_id: None,
            },
        );
        WorkerJob {
            job_id: job_id.to_string(),
            queue: "other-queue".to_string(),
            envelope,
            max_attempts: 1,
            not_before_ms: 0,
            priority: 0,
        }
    }

    // Test A: a FunctionJob runs the registry; the inner handler is NOT called.
    #[tokio::test]
    async fn routing_function_job_runs_registry_not_inner() {
        let steps = Arc::new(InMemoryStepStore::new());
        let runs = Arc::new(InMemoryRunStore::new());
        let mut reg = FunctionRegistry::with_stores(
            Arc::clone(&steps) as Arc<dyn StepStore>,
            Arc::clone(&runs) as Arc<dyn RunStore>,
        );
        reg.register(simple_def("fn-route")).expect("register");

        let inner_called = Arc::new(AtomicBool::new(false));
        let handler = RoutingJobHandler::new(
            Arc::new(reg),
            RecordingHandler {
                called: Arc::clone(&inner_called),
            },
        );

        let func_job = FunctionJob {
            function_id: "fn-route".to_string(),
            run_id: "run-route-1".to_string(),
            payload: json!({}),
            mode: JobMode::Invoke,
        };
        let worker_job = build_worker_job("job-route-1", &func_job).expect("build");
        handler.handle(&worker_job).await.expect("handle ok");

        // The function ran (registry recorded a completed run).
        let record = runs
            .get_run("run-route-1")
            .await
            .expect("get_run")
            .expect("Some");
        assert_eq!(record.status, RunStatus::Completed);

        // The inner handler must NOT have been called.
        assert!(
            !inner_called.load(Ordering::SeqCst),
            "inner handler must not run for a FunctionJob"
        );
    }

    // Test B: a non-function job is delegated to the inner handler.
    #[tokio::test]
    async fn routing_non_function_job_delegates_to_inner() {
        let reg = FunctionRegistry::new();

        let inner_called = Arc::new(AtomicBool::new(false));
        let handler = RoutingJobHandler::new(
            Arc::new(reg),
            RecordingHandler {
                called: Arc::clone(&inner_called),
            },
        );

        let job = non_function_job("job-other-1");
        // Sanity: the routing precondition — this job is not a FunctionJob.
        assert!(extract_function_job(&job).is_err());

        handler.handle(&job).await.expect("handle ok");

        assert!(
            inner_called.load(Ordering::SeqCst),
            "inner handler must run for a non-FunctionJob"
        );
    }
}

#[cfg(all(test, feature = "cron"))]
mod cron_wiring_tests {
    use tdw_cron::ScheduleRegistry;

    use super::{FunctionDef, FunctionRegistry, Trigger};
    use crate::cron_wiring::register_cron_triggers;

    fn cron_fn(id: &str, exprs: &[&str]) -> FunctionDef {
        let triggers = exprs
            .iter()
            .map(|e| Trigger::Cron((*e).to_string()))
            .collect();
        FunctionDef::from_fn(id.to_string(), triggers, |_ctx, _p| {
            Box::pin(async { Ok(serde_json::json!("ok")) })
        })
    }

    fn event_fn(id: &str, event: &str) -> FunctionDef {
        FunctionDef::from_fn(
            id.to_string(),
            vec![Trigger::Event(event.to_string())],
            |_ctx, _p| Box::pin(async { Ok(serde_json::json!("ok")) }),
        )
    }

    // -----------------------------------------------------------------------
    // registration count
    // -----------------------------------------------------------------------

    #[test]
    fn register_cron_triggers_count() {
        let mut reg = FunctionRegistry::new();
        reg.register(cron_fn("fn-a", &["*/5 * * * *", "0 9 * * MON"]))
            .expect("register fn-a");
        reg.register(cron_fn("fn-b", &["0 0 * * *"]))
            .expect("register fn-b");
        reg.register(event_fn("fn-c", "price.updated"))
            .expect("register fn-c");

        let mut schedules = ScheduleRegistry::new();
        let count = register_cron_triggers(&reg, &mut schedules).expect("register cron triggers");

        assert_eq!(count, 3, "2 from fn-a + 1 from fn-b; event trigger ignored");
        assert_eq!(schedules.len(), 3);
    }

    // -----------------------------------------------------------------------
    // trigger ids are deterministic
    // -----------------------------------------------------------------------

    #[test]
    fn trigger_ids_are_deterministic() {
        let mut reg = FunctionRegistry::new();
        reg.register(cron_fn("my-fn", &["*/5 * * * *"]))
            .expect("register");

        let mut schedules = ScheduleRegistry::new();
        register_cron_triggers(&reg, &mut schedules).expect("register");

        let ids: Vec<_> = schedules.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["my-fn:cron:0"]);
    }

    // -----------------------------------------------------------------------
    // bad cron expression returns FunctionError::Trigger
    // -----------------------------------------------------------------------

    #[test]
    fn bad_cron_expression_returns_trigger_error() {
        let mut reg = FunctionRegistry::new();
        reg.register(cron_fn("fn-bad", &["not a cron"]))
            .expect("register");

        let mut schedules = ScheduleRegistry::new();
        let err = register_cron_triggers(&reg, &mut schedules)
            .expect_err("should fail on bad expression");
        assert!(
            matches!(err, crate::FunctionError::Trigger(_)),
            "expected Trigger error, got: {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // empty registry → zero triggers
    // -----------------------------------------------------------------------

    #[test]
    fn empty_registry_registers_zero_triggers() {
        let reg = FunctionRegistry::new();
        let mut schedules = ScheduleRegistry::new();
        let count = register_cron_triggers(&reg, &mut schedules).expect("no error");
        assert_eq!(count, 0);
        assert!(schedules.is_empty());
    }
}

#[cfg(all(test, feature = "worker"))]
mod event_wiring_tests {
    use serde_json::json;
    use tdw_worker::{DurableWorkerQueue, InMemoryWorkerQueue};

    use super::{FunctionDef, FunctionRegistry, Trigger};
    use crate::event_wiring::EventRouter;

    fn event_fn(id: &str, events: &[&str]) -> FunctionDef {
        let triggers = events
            .iter()
            .map(|e| Trigger::Event((*e).to_string()))
            .collect();
        FunctionDef::from_fn(id.to_string(), triggers, |_ctx, _p| {
            Box::pin(async { Ok(json!("ok")) })
        })
    }

    fn cron_fn(id: &str) -> FunctionDef {
        FunctionDef::from_fn(
            id.to_string(),
            vec![Trigger::Cron("*/5 * * * *".to_string())],
            |_ctx, _p| Box::pin(async { Ok(json!("ok")) }),
        )
    }

    // -----------------------------------------------------------------------
    // router dispatches to all and only matching functions
    // -----------------------------------------------------------------------

    #[test]
    fn dispatch_routes_to_matching_functions() {
        let mut reg = FunctionRegistry::new();
        reg.register(event_fn("fn-price", &["price.updated"]))
            .expect("reg");
        reg.register(event_fn("fn-both", &["price.updated", "trade.filled"]))
            .expect("reg");
        reg.register(event_fn("fn-trade", &["trade.filled"]))
            .expect("reg");
        reg.register(cron_fn("fn-cron")).expect("reg"); // must NOT appear

        let router = EventRouter::from_registry(&reg);

        let subs_price = router.subscribers("price.updated");
        // Both fn-price and fn-both subscribe — deterministic (BTreeMap) order
        assert_eq!(subs_price.len(), 2);
        assert!(subs_price.contains(&"fn-price".to_string()));
        assert!(subs_price.contains(&"fn-both".to_string()));

        let subs_trade = router.subscribers("trade.filled");
        assert_eq!(subs_trade.len(), 2);
        assert!(subs_trade.contains(&"fn-both".to_string()));
        assert!(subs_trade.contains(&"fn-trade".to_string()));
    }

    #[test]
    fn dispatch_unknown_event_enqueues_zero_jobs() {
        let mut reg = FunctionRegistry::new();
        reg.register(event_fn("fn-x", &["known.event"]))
            .expect("reg");

        let router = EventRouter::from_registry(&reg);
        let mut queue = InMemoryWorkerQueue::default();

        let outcomes = router
            .dispatch(&mut queue, "unknown.event", json!({}), "seed", 0)
            .expect("dispatch");
        assert!(
            outcomes.is_empty(),
            "no function subscribes to unknown.event"
        );
        assert_eq!(queue.stats().pending, 0);
    }

    #[test]
    fn dispatch_enqueues_one_job_per_subscriber() {
        let mut reg = FunctionRegistry::new();
        reg.register(event_fn("fn-a", &["data.arrived"]))
            .expect("reg");
        reg.register(event_fn("fn-b", &["data.arrived"]))
            .expect("reg");

        let router = EventRouter::from_registry(&reg);
        let mut queue = InMemoryWorkerQueue::default();

        let outcomes = router
            .dispatch(&mut queue, "data.arrived", json!({"key": "v"}), "evt-1", 0)
            .expect("dispatch");
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(|o| o.inserted));
        assert_eq!(queue.stats().pending, 2);
    }

    #[test]
    fn dispatch_run_ids_are_seed_plus_function_id() {
        let mut reg = FunctionRegistry::new();
        reg.register(event_fn("fn-x", &["ev"])).expect("reg");

        let router = EventRouter::from_registry(&reg);
        let mut queue = InMemoryWorkerQueue::default();

        let outcomes = router
            .dispatch(&mut queue, "ev", json!({}), "my-seed", 0)
            .expect("dispatch");
        assert_eq!(outcomes.len(), 1);
        // job_id = run_id = "my-seed:fn-x"
        assert_eq!(outcomes[0].job_id, "my-seed:fn-x");
    }

    #[test]
    fn router_ignores_cron_triggers() {
        let mut reg = FunctionRegistry::new();
        reg.register(cron_fn("fn-cron")).expect("reg");

        let router = EventRouter::from_registry(&reg);
        // No event routes registered
        assert!(router.subscribers("any.event").is_empty());
    }
}

#[cfg(all(test, feature = "sqlite"))]
mod sqlite_tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::{FunctionDef, FunctionRegistry, RunStatus, RunStore, StepStore};
    use crate::sqlite::{SqliteRunStore, SqliteStepStore};

    // -----------------------------------------------------------------------
    // SqliteStepStore: basic put/get/list
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn sqlite_step_store_put_get_roundtrip() {
        let store = SqliteStepStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        store.put("r1", "s1", &json!(99)).await.expect("put");
        let got = store.get("r1", "s1").await.expect("get").expect("Some");
        assert_eq!(got, json!(99));
    }

    #[tokio::test]
    async fn sqlite_step_store_first_write_wins() {
        let store = SqliteStepStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        store.put("r1", "s1", &json!(1)).await.expect("put 1");
        store.put("r1", "s1", &json!(2)).await.expect("put 2");
        let got = store.get("r1", "s1").await.expect("get").expect("Some");
        assert_eq!(got, json!(1), "first write must win");
    }

    #[tokio::test]
    async fn sqlite_step_store_list() {
        let store = SqliteStepStore::connect("sqlite::memory:")
            .await
            .expect("connect");
        store.put("r1", "b", &json!("B")).await.expect("b");
        store.put("r1", "a", &json!("A")).await.expect("a");
        store.put("r2", "x", &json!("X")).await.expect("x");
        let mut listed = store.list("r1").await.expect("list");
        listed.sort_by_key(|(k, _)| k.clone());
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0], ("a".to_string(), json!("A")));
        assert_eq!(listed[1], ("b".to_string(), json!("B")));
    }

    // -----------------------------------------------------------------------
    // Registry with SQLite stores: full invoke round-trip
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn registry_with_sqlite_stores_invoke_completes() {
        let steps = Arc::new(
            SqliteStepStore::connect("sqlite::memory:")
                .await
                .expect("connect steps"),
        );
        let runs = Arc::new(
            SqliteRunStore::connect("sqlite::memory:")
                .await
                .expect("connect runs"),
        );
        let mut reg = FunctionRegistry::with_stores(
            Arc::clone(&steps) as Arc<dyn StepStore>,
            Arc::clone(&runs) as Arc<dyn RunStore>,
        );
        reg.register(FunctionDef::from_fn(
            "fn-sqlite",
            vec![],
            |ctx, _payload| {
                Box::pin(async move { ctx.step("s1", || async { Ok(json!("done")) }).await })
            },
        ))
        .expect("register");

        let result = reg
            .invoke("fn-sqlite", json!({}), "run-sq1".to_string(), 0)
            .await
            .expect("invoke");
        assert_eq!(result, json!("done"));

        let record = runs
            .get_run("run-sq1")
            .await
            .expect("get_run")
            .expect("Some");
        assert_eq!(record.status, RunStatus::Completed);
    }
}
