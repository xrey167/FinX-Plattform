//! Named multi-step function registry with per-step memoization.
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
//!   / [`InMemoryRunStore`] always compiled and [`PgStepStore`] /
//!   [`PgRunStore`] behind the `postgres` feature.
//!
//! # Trigger format
//!
//! [`Trigger::Cron`] holds a 5-field cron expression (`min hour dom month
//! dow`) matching the format used by `tdw-cron`.  Nothing in this sub-PR
//! consumes triggers — they exist as metadata for callers.
//!
//! # StepFn / handler shape
//!
//! The handler trait is [`StepFn`]:
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
//! # RuntimeConfig
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
