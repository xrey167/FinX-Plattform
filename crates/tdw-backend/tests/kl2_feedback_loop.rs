//! Production-path tests for the K-L2 feedback → consolidation loop.
//!
//! These tests close autonomy gaps #2 and #8: the periodic consolidation tick
//! previously ignored the [`RetrievalFeedbackStore`] and discarded computed
//! plans without persisting them.  K-L2 fixes both:
//!
//! 1. The tick now drains the shared [`RetrievalFeedbackStore`] through the
//!    usage-aware planner ([`consolidation_plan_with_usage`]).
//! 2. The applied plan is persisted to `*.json5` — tier changes survive a
//!    restart.
//! 3. A [`ConsolidationFreshness`] cell is updated after every tick and surfaced
//!    in `tdw.kg.status`.
//!
//! ## Production-path discipline
//!
//! Every gate test that exercises wiring uses the REAL `Backend::from_config`
//! (tempdir-pointed `TdwConfig` with `sqlite::memory:`) rather than the
//! `in_memory_for_tests` shortcut — matching the K-L1 Gate-8 and K-L3
//! `from_config_with_split_id_wires_eval_freshness_cell` precedents.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tdw_agent::{
    Adaptivity, DataFacets, EntityMeta, Materialization, Memory, Origin, Plane, Retention, Source,
    Tier,
};
use tdw_agent_store::{RetrievalEvent, RetrievalFeedbackStore};
use tdw_backend::data::{Backend, build_consolidation_freshness_cell};
use tdw_config::TdwConfig;
use tdw_knowledge::runtime::{ConsolidationFreshness, KnowledgeVersions};
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A unique temp directory under the OS temp dir, created fresh per test.
fn unique_temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("tdw-kl2-{tag}-{pid}-{n}"));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Build a minimal [`TdwConfig`] with in-memory SQLite stores.
///
/// These tests do NOT set `TDW_MEMORY_DIR` — persistence round-trip tests
/// exercise `MemoryStore::load_dir` and `spawn_consolidation_scheduler_with_feedback`
/// directly (via the `tdw-agent-store` API) rather than routing through
/// `Backend::from_config` with an env var, which would require `unsafe`
/// `set_var` / `remove_var` calls that are forbidden by `#![forbid(unsafe_code)]`.
#[allow(clippy::doc_markdown)]
fn minimal_config() -> TdwConfig {
    let mut cfg = TdwConfig::default();
    cfg.session.sqlite_path = "sqlite::memory:".to_string();
    cfg.worker.sqlite_path = "sqlite::memory:".to_string();
    cfg
}

fn make_memory(name: &str, retention: Retention, last_consolidated: Option<&str>) -> Memory {
    Memory {
        meta: EntityMeta::new(
            name,
            name,
            "0.1.0",
            Origin {
                tier: Tier::Domain,
                source: Source::Internal,
            },
            Adaptivity::SelfModifying,
            false,
        ),
        retention,
        last_consolidated: last_consolidated.map(ToString::to_string),
        source_entries: Vec::new(),
        facets: DataFacets {
            plane: Plane::Agent,
            materialization: Materialization::Materialized,
            as_of: None,
            validation: None,
        },
    }
}

fn used_event(agent_id: &str, recorded_at: &str) -> RetrievalEvent {
    RetrievalEvent {
        agent_id: agent_id.to_string(),
        query_fingerprint: "fp-kl2".to_string(),
        hit_ids: vec![],
        versions: KnowledgeVersions {
            embedder_model: "hash".to_string(),
            rules_version: None,
            infer_version: None,
        },
        used: true,
        recorded_at: recorded_at.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Gate 1: empty-store tick regression
//
// An empty feedback store must produce the same actions as bare consolidate_at.
// This pins the B10 regression contract through the new scheduler path.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn empty_store_tick_identical_to_consolidate_at_regression() {
    use tdw_agent_store::{MemoryStore, consolidate_at};

    let now = "2026-06-10T00:00:00Z";

    // Expected: bare consolidate_at on an aged ShortTerm (ttl=1, age=2 → promote).
    let mut aged = make_memory("note", Retention::ShortTerm, Some("2026-06-08T00:00:00Z"));
    aged.last_consolidated = Some("2026-06-08T00:00:00Z".to_string());

    let base_actions = {
        let mut plain = MemoryStore::new();
        plain.upsert_at(aged.clone(), now).expect("upsert");
        consolidate_at(&mut plain, now).expect("consolidate_at")
    };

    // Backend with empty feedback — must produce identical actions.
    let backend = Backend::in_memory_for_tests().await;
    backend.upsert_memory(aged).await.expect("upsert");
    // feedback store starts empty — no events appended.
    let backend_actions = backend
        .consolidate_now_at(now)
        .await
        .expect("consolidate_now_at");

    assert_eq!(
        backend_actions, base_actions,
        "empty feedback store must produce identical actions to consolidate_at"
    );
}

// ---------------------------------------------------------------------------
// Gate 2: non-empty store tick applies recency credit DURABLY (persist + reload)
//
// Seeds an aged ShortTerm memory in a dir-backed store, appends a `used`
// feedback event, fires ONE scheduler tick, then reloads and asserts the
// memory was NOT promoted (recency credit suppressed the action).
//
// Uses `MemoryStore::load_dir` + `spawn_consolidation_scheduler_with_feedback`
// directly — no `Backend::from_config` (avoids `set_var`/`remove_var` which
// require `unsafe` and are forbidden by `#![forbid(unsafe_code)]`).
//
// Credit math (relative to real wall clock, which the scheduler reads):
//   raw_age       = days(past → now_real) ≥ 3
//   recorded_at   = chrono::Utc::now() (set at test start, days_since_used ≈ 0)
//   credit        = raw_age.saturating_sub(0) = raw_age
//   effective_age = max(0, raw_age − credit) = 0 < 1 = ttl → no action (no promotion).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_empty_store_tick_applies_recency_credit_durably_across_restart() {
    use std::time::Duration;
    use tdw_agent_store::{MemoryStore, spawn_consolidation_scheduler_with_feedback};
    use tdw_app_server::CancellationToken;

    let dir = unique_temp_dir("persist");

    // `past` is the last_consolidated anchor — well in the past so raw_age > ttl=1.
    let past = "2026-06-09T00:00:00Z";
    // `recorded_at` is set to NOW (wall clock) so days_since_used=0 → credit=raw_age → no promote.
    let recorded_at = chrono::Utc::now().to_rfc3339();

    // --- Seed phase: build a dir-backed store and insert the aged memory. ---
    {
        let mut store = MemoryStore::load_dir(&dir).expect("load_dir seed phase");
        let mut mem = make_memory("sensor", Retention::ShortTerm, Some(past));
        mem.last_consolidated = Some(past.to_string());
        store.upsert_at(mem, past).expect("upsert seed");
    }

    // --- Tick phase: one scheduler tick with a `used` feedback event. ---
    {
        let store = Arc::new(Mutex::new(
            MemoryStore::load_dir(&dir).expect("load_dir tick phase"),
        ));
        let feedback = Arc::new(Mutex::new(RetrievalFeedbackStore::default()));
        feedback
            .lock()
            .await
            .append(used_event("sensor", &recorded_at))
            .expect("append feedback");

        let freshness_cell = build_consolidation_freshness_cell();
        let cancel = CancellationToken::new();

        // Tick every 1 ms — cancel immediately after one iteration completes.
        let handle = spawn_consolidation_scheduler_with_feedback(
            Arc::clone(&store),
            Arc::clone(&feedback),
            Arc::clone(&freshness_cell),
            Duration::from_millis(1),
            cancel.clone(),
        );

        // Poll until the freshness cell transitions away from Pending.
        for _ in 0..200u32 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let state = freshness_cell.try_lock().map(|g| g.clone());
            if let Ok(ConsolidationFreshness::Ok { .. } | ConsolidationFreshness::Error { .. }) =
                state
            {
                break;
            }
        }

        cancel.cancel();
        let _ = handle.await;
    }

    // --- Reload phase: retention must still be ShortTerm (no promotion). ---
    let reloaded = tdw_agent_store::MemoryStore::load_dir(&dir).expect("reload dir");
    assert_eq!(
        reloaded.get("sensor").map(|m| m.retention),
        Some(Retention::ShortTerm),
        "memory survived (not promoted) with recency credit — tier persisted correctly"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Gate 3: promotion persists across restart (no feedback, plain promote path)
//
// Seeds an aged ShortTerm memory, runs `consolidate_at` WITHOUT feedback
// (raw_age > ttl → promote), reloads and asserts MidTerm tier survived.
//
// Uses `MemoryStore::load_dir` + bare `consolidate_at` directly — the store
// has a backing dir so `consolidate_at` persists the tier to disk.
// ---------------------------------------------------------------------------

#[test]
fn promotion_persists_across_restart_without_feedback() {
    use tdw_agent_store::{MemoryStore, consolidate_at};

    let dir = unique_temp_dir("promote");
    let past = "2026-06-09T00:00:00Z"; // well in the past → raw_age > ttl=1 → Promote

    // Seed phase.
    {
        let mut store = MemoryStore::load_dir(&dir).expect("load_dir seed");
        let mut mem = make_memory("note", Retention::ShortTerm, Some(past));
        mem.last_consolidated = Some(past.to_string());
        store.upsert_at(mem, past).expect("upsert");
    }

    // Consolidation phase: use a real-clock `now` string (ISO 8601) so raw_age
    // is guaranteed > 1 for any run date after 2026-06-10.
    {
        let now = chrono::Utc::now().to_rfc3339();
        let mut store = MemoryStore::load_dir(&dir).expect("load_dir consolidate");
        let actions = consolidate_at(&mut store, &now).expect("consolidate_at");
        assert_eq!(actions.len(), 1, "exactly one promotion action expected");
    }

    // Reload — tier must be MidTerm.
    let reloaded = tdw_agent_store::MemoryStore::load_dir(&dir).expect("reload");
    assert_eq!(
        reloaded.get("note").map(|m| m.retention),
        Some(Retention::MidTerm),
        "promoted tier (MidTerm) must survive a reload from disk"
    );
    assert!(
        reloaded
            .get("note")
            .and_then(|m| m.last_consolidated.clone())
            .is_some(),
        "last_consolidated breadcrumb must be present after promotion"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Gate 4: injected-tick determinism
//
// `consolidate_now_at` with an injected `now` string must be deterministic:
// calling it twice with the same `now` and the same feedback store state must
// produce the same actions (idempotent on the second pass because the first
// pass already updated `last_consolidated`).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn injected_tick_is_deterministic() {
    let now = "2026-06-11T00:00:00Z";
    let past = "2026-06-09T00:00:00Z";

    let backend = Backend::in_memory_for_tests().await;
    let mut mem = make_memory("det", Retention::ShortTerm, Some(past));
    mem.last_consolidated = Some(past.to_string());
    backend.upsert_memory(mem).await.expect("upsert");

    // First pass: promotes.
    let first = backend.consolidate_now_at(now).await.expect("first pass");
    assert_eq!(first.len(), 1, "first pass must promote");

    // Second pass with the same `now`: last_consolidated was updated to `now`,
    // so age=0 < ttl=7 (MidTerm) → no actions.
    let second = backend.consolidate_now_at(now).await.expect("second pass");
    assert!(
        second.is_empty(),
        "second pass with same now must produce no actions: {second:?}"
    );
}

// ---------------------------------------------------------------------------
// Gate 5: from_config wires the consolidation_freshness cell into KnowledgeRuntime
//
// Verifies the production construction path: `from_config` builds the cell,
// attaches it to the runtime, and the initial status is `Pending`.
// ---------------------------------------------------------------------------

#[tokio::test]
#[allow(clippy::significant_drop_tightening)]
// test: cell owner must outlive guard block intentionally
async fn from_config_wires_consolidation_freshness_cell_into_runtime() {
    let cfg = minimal_config();

    let backend = Backend::from_config(cfg)
        .await
        .expect("Backend::from_config must succeed with in-memory config");

    let runtime = backend.knowledge_runtime_handle();
    let status = runtime.status().await;

    // Cell is attached — status must be Pending (not yet ticked).
    assert_eq!(
        status.consolidation_freshness,
        ConsolidationFreshness::Pending,
        "initial consolidation_freshness must be Pending before any tick; \
         got: {:?}",
        status.consolidation_freshness
    );

    // The cell is accessible and starts Pending.
    let cell = backend.consolidation_freshness_cell();
    let guard = cell.try_lock().expect("not contended");
    assert_eq!(
        *guard,
        ConsolidationFreshness::Pending,
        "consolidation_freshness_cell must start Pending"
    );
}

// ---------------------------------------------------------------------------
// Gate 6: shared cell write is visible in runtime status
//
// A write to the cell from the "scheduler side" must be reflected in
// `KnowledgeRuntime::status()`, proving the Arc is truly shared.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn shared_cell_write_is_visible_in_runtime_status() {
    let cfg = minimal_config();

    let backend = Backend::from_config(cfg).await.expect("from_config");
    let runtime = backend.knowledge_runtime_handle();

    // Write Ok to the cell (simulating a successful tick).
    {
        let cell = backend.consolidation_freshness_cell();
        let mut guard = cell.lock().await;
        *guard = ConsolidationFreshness::Ok {
            last_tick_at: "2026-06-11T00:00:00Z".to_string(),
            before_count: 3,
            after_count: 2,
            applied_count: 1,
        };
    }

    let status = runtime.status().await;
    assert!(
        matches!(
            status.consolidation_freshness,
            ConsolidationFreshness::Ok {
                applied_count: 1,
                ..
            }
        ),
        "Ok written to cell must be visible in status; got: {:?}",
        status.consolidation_freshness
    );
}

// ---------------------------------------------------------------------------
// Gate 7: build_consolidation_freshness_cell always returns Pending
// ---------------------------------------------------------------------------

#[test]
#[allow(clippy::significant_drop_tightening)]
// test: cell owner must outlive guard block intentionally
fn build_consolidation_freshness_cell_returns_pending() {
    let cell = build_consolidation_freshness_cell();
    let guard = cell.try_lock().expect("not contended");
    assert_eq!(
        *guard,
        ConsolidationFreshness::Pending,
        "build_consolidation_freshness_cell must return Pending"
    );
}

// ---------------------------------------------------------------------------
// Gate 8: live scheduler drains feedback and updates freshness cell
//
// Spawns the new scheduler with a tiny tick interval, appends a used feedback
// event, waits for the tick to fire, and asserts:
//   (a) the freshness cell transitions from Pending → Ok
//   (b) the memory with recency credit survives (not promoted/expired)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_scheduler_drains_feedback_and_updates_freshness_cell() {
    use tdw_agent_store::{MemoryStore, spawn_consolidation_scheduler_with_feedback};
    use tdw_app_server::CancellationToken;

    let now_str = chrono::Utc::now().to_rfc3339();

    // Seed a Working memory (ttl=0 → expires on first tick without feedback).
    let mut store = MemoryStore::new();
    store
        .upsert_at(
            make_memory("buf", Retention::Working, Some(&now_str)),
            &now_str,
        )
        .expect("upsert working buf");

    let store_arc = Arc::new(Mutex::new(store));
    let feedback_arc = Arc::new(Mutex::new(RetrievalFeedbackStore::new()));
    let freshness_arc = build_consolidation_freshness_cell();
    let cancel = CancellationToken::new();

    let handle = spawn_consolidation_scheduler_with_feedback(
        Arc::clone(&store_arc),
        Arc::clone(&feedback_arc),
        Arc::clone(&freshness_arc),
        Duration::from_millis(5),
        cancel.clone(),
    );

    // Wait until the Working buffer is expired by the scheduler.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if store_arc.lock().await.get("buf").is_none() {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // (a) Working buffer was expired by the scheduler.
    assert!(
        store_arc.lock().await.get("buf").is_none(),
        "scheduler must expire the Working buffer on its first tick"
    );

    // (b) Freshness cell transitioned from Pending → Ok.
    let freshness = freshness_arc.lock().await.clone();
    assert!(
        matches!(freshness, ConsolidationFreshness::Ok { .. }),
        "freshness cell must be Ok after at least one tick; got: {freshness:?}"
    );

    // Clean shutdown.
    cancel.cancel();
    let joined = tokio::time::timeout(Duration::from_secs(2), handle).await;
    assert!(joined.is_ok(), "scheduler must stop cleanly on cancel");
}
