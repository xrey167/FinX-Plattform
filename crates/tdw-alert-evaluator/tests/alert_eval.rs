//! Integration tests for the price-alert evaluation function.
//!
//! All tests use:
//! - [`tdw_alerts::InMemoryAlertStore`] — in-process, no I/O.
//! - [`tdw_alert_evaluator::StaticQuoteSource`] — deterministic pre-loaded map.
//! - `CountingNotifier` — records every `notify` call for assertion.
//!
//! The [`FunctionRegistry`] is driven directly via
//! [`FunctionRegistry::invoke`] / [`FunctionRegistry::resume`] with a fixed
//! `now_ms` so the clock never influences test results.

use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};

use async_trait::async_trait;
use tdw_alert_evaluator::{
    AlertEvalDeps, AlertNotifier, NoopNotifier, QuoteSource, StaticQuoteSource,
    alert_evaluation_function,
};
use tdw_alerts::{AlertDirection, AlertStore, InMemoryAlertStore, NewAlert, PriceAlert};
use tdw_domain::QuoteSnapshot;
use tdw_functions::{
    FunctionRegistry, InMemoryRunStore, InMemoryStepStore, RunStatus, RunStore, StepStore,
};

// ---------------------------------------------------------------------------
// Shared constants
// ---------------------------------------------------------------------------

/// Fixed "current time" for all tests — far enough in the future that alerts
/// created with default TTL (90 days) are not expired.
const NOW: i64 = 2_000_000_000_000_i64;

/// A timestamp that is well before NOW but still valid for alert creation.
const CREATED_AT: i64 = NOW - 1_000_000;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Build a minimal [`PriceAlert`] fixture.
fn make_alert(id: &str, symbol: &str, target_price: f64, condition: AlertDirection) -> PriceAlert {
    PriceAlert::new(
        NewAlert {
            owner_id: "user@example.com".to_string(),
            symbol: symbol.to_string(),
            target_price,
            condition,
            expires_at_ms: Some(NOW + 1_000_000), // expires well after NOW
        },
        id.to_string(),
        CREATED_AT,
    )
}

/// Build a [`QuoteSnapshot`] at `price` for `symbol`.
fn make_quote(symbol: &str, price: f64) -> QuoteSnapshot {
    QuoteSnapshot {
        symbol: symbol.to_string(),
        current_price: price,
        change: 0.0,
        change_percent: 0.0,
        prev_close: price,
        ts_ms: NOW,
    }
}

// ---------------------------------------------------------------------------
// CountingNotifier — records notify calls
// ---------------------------------------------------------------------------

/// A test [`AlertNotifier`] that records `(alert_id, symbol)` for each call.
#[derive(Debug, Default, Clone)]
struct CountingNotifier {
    calls: Arc<Mutex<Vec<(String, String)>>>,
}

impl CountingNotifier {
    fn new() -> Self {
        Self::default()
    }

    /// Return the number of `notify` calls made so far.
    fn call_count(&self) -> usize {
        self.calls.lock().expect("mutex").len()
    }

    /// Return whether `notify` was called for `alert_id`.
    fn was_called_for(&self, alert_id: &str) -> bool {
        self.calls
            .lock()
            .expect("mutex")
            .iter()
            .any(|(id, _)| id == alert_id)
    }
}

#[async_trait]
impl AlertNotifier for CountingNotifier {
    async fn notify(&self, alert: &PriceAlert, quote: &QuoteSnapshot) -> Result<(), String> {
        self.calls
            .lock()
            .expect("mutex")
            .push((alert.id.clone(), quote.symbol.clone()));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// FailingNotifier — always returns Err
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone, Copy)]
struct FailingNotifier;

#[async_trait]
impl AlertNotifier for FailingNotifier {
    async fn notify(&self, _alert: &PriceAlert, _quote: &QuoteSnapshot) -> Result<(), String> {
        Err("simulated notification failure".to_string())
    }
}

// ---------------------------------------------------------------------------
// CountingQuoteSource — wraps StaticQuoteSource and counts calls
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct CountingQuoteSource {
    inner: StaticQuoteSource,
    calls: Arc<AtomicU32>,
}

impl CountingQuoteSource {
    fn new(quotes: BTreeMap<String, QuoteSnapshot>) -> Self {
        Self {
            inner: StaticQuoteSource::new(quotes),
            calls: Arc::new(AtomicU32::new(0)),
        }
    }

    fn call_count(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl QuoteSource for CountingQuoteSource {
    async fn quote(&self, symbol: &str) -> Result<QuoteSnapshot, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.quote(symbol).await
    }
}

// ---------------------------------------------------------------------------
// Registry builder
// ---------------------------------------------------------------------------

fn build_registry(deps: AlertEvalDeps) -> FunctionRegistry {
    let mut reg = FunctionRegistry::new();
    reg.register(alert_evaluation_function(deps))
        .expect("register price-alert-evaluation");
    reg
}

// ---------------------------------------------------------------------------
// Helper: invoke the function with a fixed run_id and our fixed NOW clock.
// The handler internally reads SystemTime::now() but the step results do not
// depend on wall time — they use the store's in-memory alert data which we
// control.  We pass NOW as the registry `now_ms` (used only for RunRecord
// timestamps, not for alert eligibility inside the handler).
// ---------------------------------------------------------------------------

async fn invoke(reg: &FunctionRegistry, run_id: &str) -> serde_json::Value {
    reg.invoke(
        "price-alert-evaluation",
        serde_json::json!({}),
        run_id.to_string(),
        NOW,
    )
    .await
    .expect("invoke")
}

// ---------------------------------------------------------------------------
// Test 1: happy path — one Above fires, one Below does not
// ---------------------------------------------------------------------------

#[tokio::test]
async fn happy_path_above_fires_below_does_not() {
    let store = Arc::new(InMemoryAlertStore::new());
    let notifier = CountingNotifier::new();
    let notifier_arc: Arc<dyn AlertNotifier> = Arc::new(notifier.clone());

    // a1: AAPL Above 150 — current 160 → fires
    store
        .insert(&make_alert("a1", "AAPL", 150.0, AlertDirection::Above))
        .await
        .expect("insert a1");
    // a2: MSFT Below 200 — current 210 → does NOT fire
    store
        .insert(&make_alert("a2", "MSFT", 200.0, AlertDirection::Below))
        .await
        .expect("insert a2");

    let mut quotes = BTreeMap::new();
    quotes.insert("AAPL".to_string(), make_quote("AAPL", 160.0));
    quotes.insert("MSFT".to_string(), make_quote("MSFT", 210.0));

    let deps = AlertEvalDeps {
        alerts: Arc::clone(&store) as Arc<dyn AlertStore>,
        quotes: Arc::new(StaticQuoteSource::new(quotes)),
        notifier: Arc::clone(&notifier_arc),
    };

    let result = invoke(&build_registry(deps), "run-happy").await;
    assert_eq!(result["processed"], 2, "processed");
    assert_eq!(result["triggered"], 1, "triggered");

    // Notifier called exactly once for a1
    assert_eq!(notifier.call_count(), 1);
    assert!(notifier.was_called_for("a1"));
    assert!(!notifier.was_called_for("a2"));

    // a1 is marked fired in store
    let eligible = store.list_eligible(NOW).await.expect("list_eligible");
    assert!(
        eligible.iter().all(|a| a.id != "a1"),
        "a1 should no longer be eligible"
    );
}

// ---------------------------------------------------------------------------
// Test 2: empty eligible → {0, 0}, quote source never called
// ---------------------------------------------------------------------------

#[tokio::test]
async fn empty_eligible_returns_zeros_and_skips_quote_source() {
    let store = Arc::new(InMemoryAlertStore::new()); // empty

    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = Arc::clone(&counter);

    // A quote source that panics if called — using CountingQuoteSource which
    // returns Err for any symbol not in its map, and we count calls.
    let counting_src = CountingQuoteSource::new(BTreeMap::new());
    let counting_src_arc = Arc::new(counting_src.clone());

    let deps = AlertEvalDeps {
        alerts: Arc::clone(&store) as Arc<dyn AlertStore>,
        quotes: Arc::clone(&counting_src_arc) as Arc<dyn QuoteSource>,
        notifier: Arc::new(NoopNotifier),
    };

    let result = invoke(&build_registry(deps), "run-empty").await;
    assert_eq!(result["processed"], 0);
    assert_eq!(result["triggered"], 0);

    // quote source must NOT have been called
    assert_eq!(
        counting_src.call_count(),
        0,
        "quote source should not be called on empty eligible set"
    );
    drop(counter_clone); // silence unused warning
}

// ---------------------------------------------------------------------------
// Test 3: missing quote symbol → alert skipped, not fired
// ---------------------------------------------------------------------------

#[tokio::test]
async fn missing_quote_symbol_skips_alert() {
    let store = Arc::new(InMemoryAlertStore::new());
    store
        .insert(&make_alert("b1", "UNKNOWN", 100.0, AlertDirection::Above))
        .await
        .expect("insert b1");

    // No quote for UNKNOWN
    let deps = AlertEvalDeps {
        alerts: Arc::clone(&store) as Arc<dyn AlertStore>,
        quotes: Arc::new(StaticQuoteSource::new(BTreeMap::new())),
        notifier: Arc::new(NoopNotifier),
    };

    let result = invoke(&build_registry(deps), "run-missing-quote").await;
    assert_eq!(result["processed"], 1);
    assert_eq!(result["triggered"], 0, "no trigger when quote is missing");

    // Alert should still be eligible
    let eligible = store.list_eligible(NOW).await.expect("eligible");
    assert_eq!(eligible.len(), 1, "b1 should remain eligible");
}

// ---------------------------------------------------------------------------
// Test 4: boundary equality — Above fires at exactly target
// ---------------------------------------------------------------------------

#[tokio::test]
async fn above_fires_at_exactly_target_price() {
    let store = Arc::new(InMemoryAlertStore::new());
    let target = 150.0_f64;
    store
        .insert(&make_alert("eq1", "AAPL", target, AlertDirection::Above))
        .await
        .expect("insert eq1");

    let mut quotes = BTreeMap::new();
    quotes.insert("AAPL".to_string(), make_quote("AAPL", target)); // exactly at target

    let deps = AlertEvalDeps {
        alerts: Arc::clone(&store) as Arc<dyn AlertStore>,
        quotes: Arc::new(StaticQuoteSource::new(quotes)),
        notifier: Arc::new(NoopNotifier),
    };

    let result = invoke(&build_registry(deps), "run-above-eq").await;
    assert_eq!(
        result["triggered"], 1,
        "Above must fire when price == target"
    );
}

// ---------------------------------------------------------------------------
// Test 5: boundary equality — Below fires at exactly target
// ---------------------------------------------------------------------------

#[tokio::test]
async fn below_fires_at_exactly_target_price() {
    let store = Arc::new(InMemoryAlertStore::new());
    let target = 200.0_f64;
    store
        .insert(&make_alert("eq2", "MSFT", target, AlertDirection::Below))
        .await
        .expect("insert eq2");

    let mut quotes = BTreeMap::new();
    quotes.insert("MSFT".to_string(), make_quote("MSFT", target)); // exactly at target

    let deps = AlertEvalDeps {
        alerts: Arc::clone(&store) as Arc<dyn AlertStore>,
        quotes: Arc::new(StaticQuoteSource::new(quotes)),
        notifier: Arc::new(NoopNotifier),
    };

    let result = invoke(&build_registry(deps), "run-below-eq").await;
    assert_eq!(
        result["triggered"], 1,
        "Below must fire when price == target"
    );
}

// ---------------------------------------------------------------------------
// Test 6: fire-once — mark_fired returns false (pre-fired) → notifier not called
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fire_once_pre_fired_alert_does_not_call_notifier() {
    let store = Arc::new(InMemoryAlertStore::new());
    let alert = make_alert("fo1", "AAPL", 100.0, AlertDirection::Above);
    store.insert(&alert).await.expect("insert");

    // Pre-fire the alert to simulate a racing run having won already.
    // We use a timestamp slightly before NOW so it's eligible for list_eligible
    // but mark_fired at NOW - 1 makes it triggered+inactive.
    // Actually mark_fired requires NOW > created_at and < expires_at.
    // Since expires_at = NOW + 1_000_000, mark_fired(NOW - 1) should succeed.
    let pre_fired = store.mark_fired("fo1", NOW - 1).await.expect("pre-fire");
    assert!(pre_fired, "pre-fire should succeed");

    // Now the alert is triggered+inactive — list_eligible returns empty.
    let notifier = CountingNotifier::new();
    let mut quotes = BTreeMap::new();
    quotes.insert("AAPL".to_string(), make_quote("AAPL", 200.0));

    let deps = AlertEvalDeps {
        alerts: Arc::clone(&store) as Arc<dyn AlertStore>,
        quotes: Arc::new(StaticQuoteSource::new(quotes)),
        notifier: Arc::new(notifier.clone()) as Arc<dyn AlertNotifier>,
    };

    let result = invoke(&build_registry(deps), "run-fire-once").await;
    assert_eq!(result["processed"], 0, "pre-fired alert not eligible");
    assert_eq!(result["triggered"], 0);
    assert_eq!(notifier.call_count(), 0, "notifier must not be called");
}

// ---------------------------------------------------------------------------
// Test 6b: fire-once via racing mark_fired — two parallel invocations
// simulate one losing the race inside fire_and_notify
// ---------------------------------------------------------------------------

/// A store wrapper whose `mark_fired` returns `false` for a specific id,
/// simulating a race condition where another run fired first.
struct RacingStore {
    inner: Arc<InMemoryAlertStore>,
    race_id: String,
}

#[async_trait]
impl AlertStore for RacingStore {
    async fn insert(&self, alert: &PriceAlert) -> tdw_alerts::Result<()> {
        self.inner.insert(alert).await
    }
    async fn list_by_owner(&self, owner_id: &str) -> tdw_alerts::Result<Vec<PriceAlert>> {
        self.inner.list_by_owner(owner_id).await
    }
    async fn delete_by_id(&self, id: &str) -> tdw_alerts::Result<bool> {
        self.inner.delete_by_id(id).await
    }
    async fn set_active(&self, id: &str, active: bool) -> tdw_alerts::Result<bool> {
        self.inner.set_active(id, active).await
    }
    async fn list_eligible(&self, now_ms: i64) -> tdw_alerts::Result<Vec<PriceAlert>> {
        self.inner.list_eligible(now_ms).await
    }
    async fn mark_fired(&self, id: &str, now_ms: i64) -> tdw_alerts::Result<bool> {
        if id == self.race_id {
            // Simulate another run winning the race — return false without
            // actually modifying the store.
            return Ok(false);
        }
        self.inner.mark_fired(id, now_ms).await
    }
}

#[tokio::test]
async fn fire_once_race_mark_fired_false_skips_notifier() {
    let inner = Arc::new(InMemoryAlertStore::new());
    inner
        .insert(&make_alert("race1", "ETH", 1000.0, AlertDirection::Above))
        .await
        .expect("insert");

    let racing_store = Arc::new(RacingStore {
        inner: Arc::clone(&inner),
        race_id: "race1".to_string(),
    });

    let notifier = CountingNotifier::new();
    let mut quotes = BTreeMap::new();
    quotes.insert("ETH".to_string(), make_quote("ETH", 2000.0));

    let deps = AlertEvalDeps {
        alerts: racing_store as Arc<dyn AlertStore>,
        quotes: Arc::new(StaticQuoteSource::new(quotes)),
        notifier: Arc::new(notifier.clone()) as Arc<dyn AlertNotifier>,
    };

    let result = invoke(&build_registry(deps), "run-race").await;
    // processed=1 (one eligible alert), triggered=0 (mark_fired returned false)
    assert_eq!(result["processed"], 1);
    assert_eq!(
        result["triggered"], 0,
        "mark_fired=false → triggered count must be 0"
    );
    assert_eq!(
        notifier.call_count(),
        0,
        "notifier must not be called when mark_fired=false"
    );
}

// ---------------------------------------------------------------------------
// Test 7: notifier error → run still succeeds, alert stays fired
// ---------------------------------------------------------------------------

#[tokio::test]
async fn notifier_error_does_not_fail_run() {
    let store = Arc::new(InMemoryAlertStore::new());
    store
        .insert(&make_alert("n1", "BTC", 50_000.0, AlertDirection::Above))
        .await
        .expect("insert");

    let mut quotes = BTreeMap::new();
    quotes.insert("BTC".to_string(), make_quote("BTC", 60_000.0));

    let deps = AlertEvalDeps {
        alerts: Arc::clone(&store) as Arc<dyn AlertStore>,
        quotes: Arc::new(StaticQuoteSource::new(quotes)),
        notifier: Arc::new(FailingNotifier) as Arc<dyn AlertNotifier>,
    };

    let result = invoke(&build_registry(deps), "run-notifier-err").await;
    // Run must succeed even though notifier failed.
    assert_eq!(result["triggered"], 1, "alert was fired");

    // Alert is still marked as fired in the store.
    let eligible = store.list_eligible(NOW).await.expect("eligible");
    assert!(
        eligible.is_empty(),
        "n1 should be fired and no longer eligible"
    );
}

// ---------------------------------------------------------------------------
// Test 8: memoization / crash-resume
//
// Construction:
// - First invoke uses a store whose mark_fired errors → step 4 fails.
// - Second invoke resumes the same run_id with a healthy store.
// - CountingQuoteSource proves step 2 (fetch_quotes) ran exactly ONCE across
//   both invocations.
// ---------------------------------------------------------------------------

/// A store wrapper that errors on the first `mark_fired` call, then delegates.
struct FailFirstFireStore {
    inner: Arc<InMemoryAlertStore>,
    attempt: Arc<AtomicU32>,
}

#[async_trait]
impl AlertStore for FailFirstFireStore {
    async fn insert(&self, alert: &PriceAlert) -> tdw_alerts::Result<()> {
        self.inner.insert(alert).await
    }
    async fn list_by_owner(&self, owner_id: &str) -> tdw_alerts::Result<Vec<PriceAlert>> {
        self.inner.list_by_owner(owner_id).await
    }
    async fn delete_by_id(&self, id: &str) -> tdw_alerts::Result<bool> {
        self.inner.delete_by_id(id).await
    }
    async fn set_active(&self, id: &str, active: bool) -> tdw_alerts::Result<bool> {
        self.inner.set_active(id, active).await
    }
    async fn list_eligible(&self, now_ms: i64) -> tdw_alerts::Result<Vec<PriceAlert>> {
        self.inner.list_eligible(now_ms).await
    }
    async fn mark_fired(&self, id: &str, now_ms: i64) -> tdw_alerts::Result<bool> {
        let n = self.attempt.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            return Err(tdw_alerts::AlertStoreError::Storage(
                "simulated mark_fired error on first attempt".to_string(),
            ));
        }
        self.inner.mark_fired(id, now_ms).await
    }
}

#[tokio::test]
async fn crash_resume_step4_reruns_steps1to3_skipped() {
    let inner = Arc::new(InMemoryAlertStore::new());
    inner
        .insert(&make_alert("cr1", "NVDA", 500.0, AlertDirection::Above))
        .await
        .expect("insert");

    let fail_store = Arc::new(FailFirstFireStore {
        inner: Arc::clone(&inner),
        attempt: Arc::new(AtomicU32::new(0)),
    });

    let mut quotes = BTreeMap::new();
    quotes.insert("NVDA".to_string(), make_quote("NVDA", 600.0));
    let counting_src = Arc::new(CountingQuoteSource::new(quotes));

    // Shared step/run stores so resume can see step results from the first run.
    let steps = Arc::new(InMemoryStepStore::new());
    let runs = Arc::new(InMemoryRunStore::new());

    let deps_attempt1 = AlertEvalDeps {
        alerts: Arc::clone(&fail_store) as Arc<dyn AlertStore>,
        quotes: Arc::clone(&counting_src) as Arc<dyn QuoteSource>,
        notifier: Arc::new(NoopNotifier),
    };

    let mut reg = FunctionRegistry::with_stores(
        Arc::clone(&steps) as Arc<dyn StepStore>,
        Arc::clone(&runs) as Arc<dyn RunStore>,
    );
    reg.register(alert_evaluation_function(deps_attempt1))
        .expect("register");

    // First invoke — step 4 (fire_and_notify) fails on mark_fired.
    let first_result = reg
        .invoke(
            "price-alert-evaluation",
            serde_json::json!({}),
            "run-resume".to_string(),
            NOW,
        )
        .await;
    assert!(first_result.is_err(), "first invoke should fail at step 4");

    // Quote source was called once (step 2 ran during first invoke).
    assert_eq!(
        counting_src.call_count(),
        1,
        "step 2 ran once on first invoke"
    );

    // Check run status is Failed.
    let record = runs
        .get_run("run-resume")
        .await
        .expect("get_run")
        .expect("Some");
    assert_eq!(record.status, RunStatus::Failed);

    // Now build a second registry with the SAME shared step/run stores but a
    // new healthy deps (fail_store.attempt is now 1, so mark_fired succeeds).
    let deps_attempt2 = AlertEvalDeps {
        alerts: Arc::clone(&fail_store) as Arc<dyn AlertStore>,
        quotes: Arc::clone(&counting_src) as Arc<dyn QuoteSource>,
        notifier: Arc::new(NoopNotifier),
    };

    let mut reg2 = FunctionRegistry::with_stores(
        Arc::clone(&steps) as Arc<dyn StepStore>,
        Arc::clone(&runs) as Arc<dyn RunStore>,
    );
    reg2.register(alert_evaluation_function(deps_attempt2))
        .expect("register");

    // Resume — steps 1–3 are memoised; only step 4 re-runs.
    let resume_result = reg2
        .resume("run-resume", NOW + 1)
        .await
        .expect("resume should succeed");

    assert_eq!(resume_result["triggered"], 1, "alert fired on resume");

    // Quote source still at 1 — step 2 was NOT re-executed on resume.
    assert_eq!(
        counting_src.call_count(),
        1,
        "step 2 must not re-run on resume (memoised)"
    );
}

// ---------------------------------------------------------------------------
// Test 9: distinct-symbol dedupe — 3 alerts, 2 symbols → quote source called twice
// ---------------------------------------------------------------------------

#[tokio::test]
async fn distinct_symbol_dedupe_calls_quote_source_once_per_symbol() {
    let store = Arc::new(InMemoryAlertStore::new());
    // Two alerts on AAPL, one on MSFT.
    store
        .insert(&make_alert("d1", "AAPL", 100.0, AlertDirection::Above))
        .await
        .expect("insert d1");
    store
        .insert(&make_alert("d2", "AAPL", 200.0, AlertDirection::Below))
        .await
        .expect("insert d2");
    store
        .insert(&make_alert("d3", "MSFT", 300.0, AlertDirection::Above))
        .await
        .expect("insert d3");

    let mut quotes = BTreeMap::new();
    quotes.insert("AAPL".to_string(), make_quote("AAPL", 150.0));
    quotes.insert("MSFT".to_string(), make_quote("MSFT", 400.0));
    let counting_src = Arc::new(CountingQuoteSource::new(quotes));

    let deps = AlertEvalDeps {
        alerts: Arc::clone(&store) as Arc<dyn AlertStore>,
        quotes: Arc::clone(&counting_src) as Arc<dyn QuoteSource>,
        notifier: Arc::new(NoopNotifier),
    };

    let result = invoke(&build_registry(deps), "run-dedupe").await;
    assert_eq!(result["processed"], 3);

    // Quote source must be called exactly twice (one per distinct symbol).
    assert_eq!(
        counting_src.call_count(),
        2,
        "quote source must be called once per distinct symbol"
    );
}

// ---------------------------------------------------------------------------
// Test 10: Above does NOT fire when price < target
// ---------------------------------------------------------------------------

#[tokio::test]
async fn above_does_not_fire_when_price_below_target() {
    let store = Arc::new(InMemoryAlertStore::new());
    store
        .insert(&make_alert("nd1", "AAPL", 200.0, AlertDirection::Above))
        .await
        .expect("insert");

    let mut quotes = BTreeMap::new();
    quotes.insert("AAPL".to_string(), make_quote("AAPL", 150.0)); // below target

    let deps = AlertEvalDeps {
        alerts: Arc::clone(&store) as Arc<dyn AlertStore>,
        quotes: Arc::new(StaticQuoteSource::new(quotes)),
        notifier: Arc::new(NoopNotifier),
    };

    let result = invoke(&build_registry(deps), "run-above-no-fire").await;
    assert_eq!(result["triggered"], 0);
}

// ---------------------------------------------------------------------------
// Test 11: Below does NOT fire when price > target
// ---------------------------------------------------------------------------

#[tokio::test]
async fn below_does_not_fire_when_price_above_target() {
    let store = Arc::new(InMemoryAlertStore::new());
    store
        .insert(&make_alert("nd2", "MSFT", 100.0, AlertDirection::Below))
        .await
        .expect("insert");

    let mut quotes = BTreeMap::new();
    quotes.insert("MSFT".to_string(), make_quote("MSFT", 150.0)); // above target

    let deps = AlertEvalDeps {
        alerts: Arc::clone(&store) as Arc<dyn AlertStore>,
        quotes: Arc::new(StaticQuoteSource::new(quotes)),
        notifier: Arc::new(NoopNotifier),
    };

    let result = invoke(&build_registry(deps), "run-below-no-fire").await;
    assert_eq!(result["triggered"], 0);
}

// ---------------------------------------------------------------------------
// Test 12: register helper wires the function into an existing registry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn register_helper_wires_function() {
    let store = Arc::new(InMemoryAlertStore::new());
    store
        .insert(&make_alert("reg1", "AAPL", 100.0, AlertDirection::Above))
        .await
        .expect("insert");

    let mut quotes = BTreeMap::new();
    quotes.insert("AAPL".to_string(), make_quote("AAPL", 200.0));

    let deps = AlertEvalDeps {
        alerts: Arc::clone(&store) as Arc<dyn AlertStore>,
        quotes: Arc::new(StaticQuoteSource::new(quotes)),
        notifier: Arc::new(NoopNotifier),
    };

    let mut reg = FunctionRegistry::new();
    tdw_alert_evaluator::register(&mut reg, deps).expect("register");

    let result = reg
        .invoke(
            "price-alert-evaluation",
            serde_json::json!({}),
            "run-reg".to_string(),
            NOW,
        )
        .await
        .expect("invoke");
    assert_eq!(result["triggered"], 1);
}
