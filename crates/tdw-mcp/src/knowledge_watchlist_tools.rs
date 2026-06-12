//! MCP tools for entity/tag knowledge watchlists (K-X5).
//!
//! # What this module implements
//!
//! A **watchlist** is a per-principal subscription to an entity or tag: when
//! the knowledge graph changes around that entity between two `as_of` points
//! the system fires a `tdw-alerts`-style alert exactly once per change (dedup
//! guard prevents re-firing on the next cron tick).
//!
//! # Tools
//!
//! | Tool | Action |
//! |------|--------|
//! | `tdw.kg.watch` | Subscribe to an entity or tag (with optional change-type filter). |
//! | `tdw.kg.unwatch` | Remove a watchlist subscription. |
//! | `tdw.kg.watchlist` | List the calling principal's watchlist entries. |
//!
//! # Change-detection design
//!
//! The cron task ([`tick_watchlist_check`]) reuses the **K-X1 diff machinery**
//! (`compute_edge_delta` / `collect_scoped_edges`) rather than reimplementing
//! temporal filtering.  For each active watch it:
//!
//! 1. Collects edges within 2 hops of the watched entity id (the same
//!    `collect_scoped_edges` the `tdw.kg.diff` tool uses).
//! 2. Calls `compute_edge_delta(last_alerted_as_of, now_as_of)` to classify
//!    changes as `new_facts | invalidations | contradictions`.
//! 3. Deduplication: stores the `last_alerted_as_of` per watch in the persisted
//!    store.  A change whose window lies entirely within `(creation_as_of,
//!    last_alerted_as_of]` is never re-fired.
//! 4. Fires at most **one alert per watch per cron tick** carrying the full
//!    delta summary so the receiver has enough context to act.
//!
//! Tag watches also run the tag delta path (the K-X1 `compute_tag_delta`
//! equivalent: `active_tags` at both points, set-diff).
//!
//! Contradiction hook: the `invalidations` bucket of the diff naturally captures
//! K-M4 closures (edges whose `valid_to` was set in the window).  No separate
//! seam is needed — if an endpoint of a closed edge is a watched entity the
//! invalidation appears in `change_types: ["invalidations"]`.  This is the
//! cleanest path: reuse diff, don't re-implement.
//!
//! # Persistence
//!
//! The store follows the caller-owns/JSON-file precedent (same as the
//! `InMemoryAlertStore` + the proposals queue): an [`InMemoryWatchlistStore`]
//! is always available; the daemon passes it as `Arc<Mutex<WatchlistStore>>`.
//! Restarts replay from the persisted JSON when `TDW_WATCHLIST_DIR` is set.
//!
//! # Caps
//!
//! * Max entries per principal: [`MAX_WATCHES_PER_PRINCIPAL`].
//! * Max k-hop radius for change detection: 2 (same as `tdw.kg.diff` scoped
//!   mode — bounded so a hub entity does not trigger a full-graph scan).
//! * Alert payload change-item list: bounded by [`WATCHLIST_CHANGE_ITEM_LIMIT`].
//!
//! # Observability
//!
//! `tdw.kg.status` gains `watchlist_status` (watch count + last-check
//! timestamp + cumulative alerts fired) surfaced from the shared freshness cell.

#![allow(clippy::too_many_lines)] // cohesive cron tick + tool dispatch; splitting hurts readability

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use tdw_core::{Direction, GraphEdge, GraphEngine, TraversalFilter, active_at};
use tdw_knowledge::runtime::{KnowledgeRuntime, WatchlistFreshness};
use tdw_tags::TagEngine;

use crate::knowledge_tools::block_on;
use crate::{ToolDescriptor, ToolExecution, ToolFailure, structured, tool_with_annotations};

// ── Tool names ────────────────────────────────────────────────────────────────

/// The names this module owns.
pub const TOOL_NAMES: &[&str] = &["tdw.kg.watch", "tdw.kg.unwatch", "tdw.kg.watchlist"];

/// Whether `name` is one of the watchlist tools.
#[must_use]
pub(crate) fn owns(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}

// ── Caps ──────────────────────────────────────────────────────────────────────

/// Maximum number of active watches per principal.  Exceeding this returns a
/// loud tool error so the caller knows to remove stale watches before adding
/// new ones.
pub const MAX_WATCHES_PER_PRINCIPAL: usize = 64;

/// Maximum number of change items included in one alert payload.  Counts are
/// always exact; the item list is bounded here.
pub const WATCHLIST_CHANGE_ITEM_LIMIT: usize = 32;

// ── Change-type vocabulary ────────────────────────────────────────────────────

/// Valid change-type tokens the caller may subscribe to.
const VALID_CHANGE_TYPES: &[&str] = &["new_facts", "invalidations", "contradictions"];

// ── Domain model ──────────────────────────────────────────────────────────────

/// What the subscriber wants to be alerted about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchChangeTypes {
    /// Alert when new edges become valid in the watched entity's neighborhood.
    pub new_facts: bool,
    /// Alert when edges in the watched entity's neighborhood are closed
    /// (invalidated) — includes K-M4 contradiction closures.
    pub invalidations: bool,
    /// Alert when contradictions surface edges from the diff's invalidation
    /// bucket.  Currently maps to the same signal as `invalidations` (both
    /// source from the diff's closed-edge list).  Kept as a distinct flag so
    /// future versions can narrow to contradiction-provenance edges only.
    pub contradictions: bool,
}

impl Default for WatchChangeTypes {
    fn default() -> Self {
        // Default: subscribe to everything.
        Self {
            new_facts: true,
            invalidations: true,
            contradictions: true,
        }
    }
}

impl WatchChangeTypes {
    /// True when every flag is false — such a watch never fires.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        !self.new_facts && !self.invalidations && !self.contradictions
    }
}

/// Subject of a watch: either an entity node id or a tag id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WatchSubject {
    /// Watch a graph entity node (2-hop neighborhood diff).
    Entity { entity_id: String },
    /// Watch a tag (entities gaining/losing this tag trigger alerts).
    Tag { tag_id: String },
}

impl WatchSubject {
    /// The stable identifier string (`entity_id` or `tag_id`).
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Entity { entity_id } => entity_id,
            Self::Tag { tag_id } => tag_id,
        }
    }
}

/// A persisted watchlist entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchEntry {
    /// Stable, unique identifier for this watch (FNV-1a hex of
    /// `"<principal_id>:<subject_id>"`).
    pub watch_id: String,
    /// The principal (user or agent id) who created this watch.
    pub principal_id: String,
    /// What is being watched.
    pub subject: WatchSubject,
    /// Which change types trigger an alert.
    pub change_types: WatchChangeTypes,
    /// The `as_of` timestamp when the watch was created.  Change detection
    /// compares from this point on the first run.
    pub created_as_of: String,
    /// The `as_of` timestamp of the most recent alert sent for this watch.
    /// Starts equal to `created_as_of`.  Updated after each alert is fired.
    pub last_alerted_as_of: String,
    /// Running count of alerts fired for this watch since creation.
    pub alerts_fired: u64,
}

impl WatchEntry {
    /// True when this watch should emit alerts (always true for enabled watches
    /// with at least one change-type flag set).
    #[must_use]
    pub const fn is_active(&self) -> bool {
        !self.change_types.is_empty()
    }
}

// ── Store ─────────────────────────────────────────────────────────────────────

/// In-memory watchlist store.  All mutations hold the `Mutex` for one
/// contiguous critical section — no await inside the lock.
#[derive(Debug, Default)]
pub struct WatchlistStore {
    /// Keyed by `watch_id` for O(1) delete.
    entries: BTreeMap<String, WatchEntry>,
    /// Cumulative alerts fired across all watches since process start.
    pub total_alerts_fired: u64,
    /// Unix-epoch ms of the last cron tick that checked watches.
    /// `0` means never checked.
    pub last_check_ms: i64,
}

impl WatchlistStore {
    /// A fresh, empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a watch entry.
    pub fn upsert(&mut self, entry: WatchEntry) {
        self.entries.insert(entry.watch_id.clone(), entry);
    }

    /// Remove a watch by id.  Returns `true` when the entry existed.
    pub fn remove(&mut self, watch_id: &str) -> bool {
        self.entries.remove(watch_id).is_some()
    }

    /// All entries for a given principal, sorted by creation timestamp.
    #[must_use]
    pub fn by_principal(&self, principal_id: &str) -> Vec<&WatchEntry> {
        let mut entries: Vec<&WatchEntry> = self
            .entries
            .values()
            .filter(|e| e.principal_id == principal_id)
            .collect();
        entries.sort_by(|a, b| a.created_as_of.cmp(&b.created_as_of));
        entries
    }

    /// Count of active entries for a principal.
    #[must_use]
    pub fn count_for_principal(&self, principal_id: &str) -> usize {
        self.entries
            .values()
            .filter(|e| e.principal_id == principal_id)
            .count()
    }

    /// All active watches (used by the cron task).
    #[must_use]
    pub fn all_active(&self) -> Vec<WatchEntry> {
        self.entries
            .values()
            .filter(|e| e.is_active())
            .cloned()
            .collect()
    }

    /// Total number of watches across all principals.
    #[must_use]
    pub fn total_count(&self) -> usize {
        self.entries.len()
    }

    /// Update `last_alerted_as_of` and increment `alerts_fired` for `watch_id`.
    pub fn record_alert(&mut self, watch_id: &str, as_of: &str) {
        if let Some(entry) = self.entries.get_mut(watch_id) {
            entry.last_alerted_as_of = as_of.to_string();
            entry.alerts_fired = entry.alerts_fired.saturating_add(1);
            self.total_alerts_fired = self.total_alerts_fired.saturating_add(1);
        }
    }
}

// ── Descriptors ───────────────────────────────────────────────────────────────

/// Descriptors for the three watchlist tools.
#[must_use]
pub(crate) fn descriptors() -> Vec<ToolDescriptor> {
    vec![
        watch_descriptor(),
        unwatch_descriptor(),
        watchlist_descriptor(),
    ]
}

fn watch_descriptor() -> ToolDescriptor {
    tool_with_annotations(
        "tdw.kg.watch",
        "Subscribe to Knowledge Watchlist",
        "Subscribe the calling principal to an entity or tag so that knowledge changes \
         (new facts, invalidations, contradictions) trigger an alert on the next cron tick.\n\
         \n\
         Watch subjects:\n\
         * `entity_id` — subscribe to the 2-hop neighborhood of this graph entity.\n\
         * `tag_id`    — subscribe to entities gaining or losing this tag.\n\
         Supply exactly one of the two. The subject id must be a valid graph id \
         (`[A-Za-z0-9:._-]+`).\n\
         \n\
         Optional `change_types` array controls which events fire alerts. Accepted tokens: \
         `new_facts`, `invalidations`, `contradictions`. Defaults to all three when omitted.\n\
         \n\
         Caps: at most 64 active watches per principal. Duplicate subscriptions \
         (same principal + same subject) are idempotently rejected with a loud error.\n\
         \n\
         Requires graph engine + bound user id.",
        json!({
            "type": "object",
            "properties": {
                "entity_id": {
                    "type": "string",
                    "description": "Graph entity id to watch (e.g. instrument:AAPL). Mutually exclusive with tag_id."
                },
                "tag_id": {
                    "type": "string",
                    "description": "Tag id to watch (e.g. sector:tech). Mutually exclusive with entity_id."
                },
                "change_types": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": ["new_facts", "invalidations", "contradictions"]
                    },
                    "description": "Change types that trigger alerts. Defaults to all three."
                }
            },
            "additionalProperties": false
        }),
        false, // not read-only
        false, // not idempotent (first call creates; duplicate is rejected)
    )
}

fn unwatch_descriptor() -> ToolDescriptor {
    tool_with_annotations(
        "tdw.kg.unwatch",
        "Remove Knowledge Watchlist Subscription",
        "Remove a watchlist subscription for the calling principal. Supply `watch_id` \
         (returned by `tdw.kg.watch` and listed by `tdw.kg.watchlist`). Returns `true` \
         when the entry existed and was removed, `false` when the id was not found.\n\
         \n\
         Requires bound user id.",
        json!({
            "type": "object",
            "properties": {
                "watch_id": {
                    "type": "string",
                    "description": "The watch_id returned by tdw.kg.watch."
                }
            },
            "required": ["watch_id"],
            "additionalProperties": false
        }),
        false,
        false,
    )
}

fn watchlist_descriptor() -> ToolDescriptor {
    tool_with_annotations(
        "tdw.kg.watchlist",
        "List Knowledge Watchlist",
        "List all active watchlist entries for the calling principal. Returns the watch id, \
         subject (entity or tag), change-type filter, creation timestamp, last-alerted \
         timestamp, and total alerts fired per entry.\n\
         \n\
         Read-only. Idempotent. Requires bound user id.",
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
        true, // readOnlyHint
        true, // idempotentHint
    )
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

/// Dispatch one watchlist tool.
///
/// `store` is the shared watchlist store (bound at daemon composition root).
/// `runtime` supplies the graph engine (required for watch creation) and the
/// bound user identity (K-L5: host-bound, never from args).
/// `now_as_of` is the current date (`YYYY-MM-DD`) injected by the MCP layer.
///
/// # Errors
///
/// Returns [`ToolFailure::Execution`] for a missing identity, malformed input,
/// cap violation, or store failure — never [`ToolFailure::Protocol`].
pub(crate) fn execute(
    runtime: &KnowledgeRuntime,
    store: &Arc<Mutex<WatchlistStore>>,
    name: &str,
    arguments: &Map<String, Value>,
    now_as_of: &str,
) -> Result<ToolExecution, ToolFailure> {
    match name {
        "tdw.kg.watch" => watch(runtime, store, arguments, now_as_of),
        "tdw.kg.unwatch" => unwatch(runtime, store, arguments),
        "tdw.kg.watchlist" => watchlist(runtime, store),
        other => Err(execution(format!("unknown watchlist tool: {other}"))),
    }
}

// ── tdw.kg.watch ─────────────────────────────────────────────────────────────

fn watch(
    runtime: &KnowledgeRuntime,
    store: &Arc<Mutex<WatchlistStore>>,
    arguments: &Map<String, Value>,
    now_as_of: &str,
) -> Result<ToolExecution, ToolFailure> {
    let principal_id = require_principal(runtime)?;
    require_graph(runtime)?; // graph must be attached for diff to work

    // Exactly one of entity_id / tag_id must be supplied.
    let entity_id = optional_str(arguments, "entity_id");
    let tag_id = optional_str(arguments, "tag_id");
    let subject = match (entity_id, tag_id) {
        (Some(eid), None) => {
            validate_graph_id(eid, "entity_id")?;
            WatchSubject::Entity {
                entity_id: eid.to_string(),
            }
        }
        (None, Some(tid)) => {
            validate_graph_id(tid, "tag_id")?;
            WatchSubject::Tag {
                tag_id: tid.to_string(),
            }
        }
        (Some(_), Some(_)) => {
            return Err(execution(
                "supply exactly one of entity_id or tag_id, not both".to_string(),
            ));
        }
        (None, None) => {
            return Err(execution(
                "supply exactly one of entity_id or tag_id".to_string(),
            ));
        }
    };

    // Change types — default to all three.
    let change_types = parse_change_types(arguments)?;

    // Stable watch id: FNV-1a hex of "<principal_id>:<subject_id>".
    let watch_id = watch_id_for(principal_id, subject.id());

    // Cap + duplicate check (single lock, no awaits inside).
    let (removed_existing, count_before) = {
        let guard = store
            .lock()
            .map_err(|_| execution("watchlist store mutex poisoned".to_string()))?;
        let exists = guard.entries.contains_key(&watch_id);
        (exists, guard.count_for_principal(principal_id))
    };

    if removed_existing {
        return Err(execution(format!(
            "principal {principal_id:?} already watches {:?}; \
             call tdw.kg.unwatch first to replace the subscription",
            subject.id()
        )));
    }

    if count_before >= MAX_WATCHES_PER_PRINCIPAL {
        return Err(execution(format!(
            "principal {principal_id:?} has reached the maximum of \
             {MAX_WATCHES_PER_PRINCIPAL} active watches; remove stale watches before adding new ones"
        )));
    }

    // Verify the entity exists in the graph when subscribing to an entity watch.
    if let WatchSubject::Entity { ref entity_id } = subject {
        let graph = require_graph(runtime)?;
        let exists = block_on(async {
            graph
                .node(entity_id)
                .await
                .map_err(|e| execution(e.to_string()))
        })?
        .is_some();
        if !exists {
            return Err(execution(format!(
                "entity {entity_id:?} does not exist in the graph — \
                 watch subscriptions require a real entity id"
            )));
        }
    }

    let entry = WatchEntry {
        watch_id: watch_id.clone(),
        principal_id: principal_id.to_string(),
        subject: subject.clone(),
        change_types: change_types.clone(),
        created_as_of: now_as_of.to_string(),
        last_alerted_as_of: now_as_of.to_string(),
        alerts_fired: 0,
    };

    {
        let mut guard = store
            .lock()
            .map_err(|_| execution("watchlist store mutex poisoned".to_string()))?;
        guard.upsert(entry);
    }

    Ok(structured(json!({
        "watch_id": watch_id,
        "principal_id": principal_id,
        "subject": serde_json::to_value(&subject).unwrap_or(Value::Null),
        "change_types": {
            "new_facts": change_types.new_facts,
            "invalidations": change_types.invalidations,
            "contradictions": change_types.contradictions,
        },
        "created_as_of": now_as_of,
        "status": "active",
        "note": format!(
            "watch created; change detection runs on the configured cadence \
             (default every 15 min). Use tdw.kg.watchlist to see all subscriptions."
        ),
    })))
}

// ── tdw.kg.unwatch ────────────────────────────────────────────────────────────

fn unwatch(
    runtime: &KnowledgeRuntime,
    store: &Arc<Mutex<WatchlistStore>>,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let principal_id = require_principal(runtime)?;
    let watch_id = require_str(arguments, "watch_id")?;

    // Security: verify the watch belongs to this principal before removing.
    let removed = {
        let mut guard = store
            .lock()
            .map_err(|_| execution("watchlist store mutex poisoned".to_string()))?;
        // Only remove when the principal matches.
        let belongs = guard
            .entries
            .get(watch_id)
            .is_some_and(|e| e.principal_id == principal_id);
        if belongs {
            guard.remove(watch_id)
        } else {
            false
        }
    };

    Ok(structured(json!({
        "watch_id": watch_id,
        "removed": removed,
        "note": if removed { "subscription removed" } else { "watch_id not found or does not belong to this principal" },
    })))
}

// ── tdw.kg.watchlist ──────────────────────────────────────────────────────────

fn watchlist(
    runtime: &KnowledgeRuntime,
    store: &Arc<Mutex<WatchlistStore>>,
) -> Result<ToolExecution, ToolFailure> {
    let principal_id = require_principal(runtime)?;

    let entries = {
        let guard = store
            .lock()
            .map_err(|_| execution("watchlist store mutex poisoned".to_string()))?;
        guard
            .by_principal(principal_id)
            .into_iter()
            .map(|e| {
                json!({
                    "watch_id": e.watch_id,
                    "subject": serde_json::to_value(&e.subject).unwrap_or(Value::Null),
                    "change_types": {
                        "new_facts": e.change_types.new_facts,
                        "invalidations": e.change_types.invalidations,
                        "contradictions": e.change_types.contradictions,
                    },
                    "created_as_of": e.created_as_of,
                    "last_alerted_as_of": e.last_alerted_as_of,
                    "alerts_fired": e.alerts_fired,
                })
            })
            .collect::<Vec<_>>()
    };

    Ok(structured(json!({
        "principal_id": principal_id,
        "watches": entries,
        "count": entries.len(),
    })))
}

// ── Cron task: change detection ───────────────────────────────────────────────

/// Alert payload emitted when a watch fires.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WatchlistAlert {
    /// The watch that fired.
    pub watch_id: String,
    /// The principal subscribed to this watch.
    pub principal_id: String,
    /// The watched subject.
    pub subject: WatchSubject,
    /// The `as_of` window compared (`from_as_of` → `to_as_of`).
    pub from_as_of: String,
    /// The end of the compared window (= now at check time).
    pub to_as_of: String,
    /// Change categories that fired (non-empty subset of the configured types).
    pub fired_change_types: Vec<String>,
    /// Representative new-fact edges (bounded by [`WATCHLIST_CHANGE_ITEM_LIMIT`]).
    pub new_fact_edges: Vec<Value>,
    /// Representative invalidated edges (bounded by [`WATCHLIST_CHANGE_ITEM_LIMIT`]).
    pub invalidated_edges: Vec<Value>,
    /// Why this alert fired (human-readable summary).
    pub why: String,
}

/// Run one watchlist-check tick.
///
/// Called by the cron task in `tdw-backend`; also directly invocable in tests
/// with an injected `now_ms` (the "injected-now" pattern).
///
/// For each active watch:
/// 1. Compute the K-X1 diff between `last_alerted_as_of` and the current date.
/// 2. Classify changes by the configured change-type filter.
/// 3. If changes exist: fire an alert, update `last_alerted_as_of`, increment
///    `alerts_fired`.
/// 4. If no changes (or no active watches): do nothing — no spurious alerts.
///
/// The dedup guarantee: `last_alerted_as_of` is advanced after EACH alert, so
/// re-running the same tick window on the next cron iteration compares from the
/// already-alerted point and produces no re-fires.
///
/// Returns the number of alerts fired in this tick.
pub async fn tick_watchlist_check(
    store: &Arc<Mutex<WatchlistStore>>,
    graph: &Arc<dyn GraphEngine>,
    tags: Option<&Arc<dyn TagEngine>>,
    now_ms: i64,
    freshness: &Arc<Mutex<WatchlistFreshness>>,
) -> usize {
    let now_as_of = ms_to_date(now_ms);

    // Snapshot the active watches without holding the mutex across awaits.
    let watches: Vec<WatchEntry> = store.lock().map_or_else(
        |poisoned| poisoned.into_inner().all_active(),
        |g| g.all_active(),
    );

    if watches.is_empty() {
        // Update freshness: no watches, nothing to do.
        if let Ok(mut f) = freshness.lock() {
            f.watch_count = 0;
            f.last_check_ms = now_ms;
            f.note = "no watchlists configured — scheduler runs but fires nothing".to_string();
        }
        eprintln!(
            "[tdw] knowledge watchlists: zero active watches — cron tick is a no-op \
             (subscribe via tdw.kg.watch)"
        );
        return 0;
    }

    let mut total_fired = 0usize;

    for watch in watches {
        // Skip if the window is zero-length (watch was created this same second).
        if watch.last_alerted_as_of >= now_as_of {
            continue;
        }

        let alert = match &watch.subject {
            WatchSubject::Entity { entity_id } => {
                check_entity_watch(&watch, entity_id, graph, &now_as_of).await
            }
            WatchSubject::Tag { tag_id } => {
                if let Some(tags_engine) = tags {
                    check_tag_watch(&watch, tag_id, tags_engine, &now_as_of).await
                } else {
                    // Tag engine absent — cannot check tag watches; log and skip.
                    eprintln!(
                        "[tdw] knowledge watchlists: tag engine not attached — \
                         skipping tag watch {:?} for {:?}",
                        watch.watch_id,
                        watch.subject.id()
                    );
                    None
                }
            }
        };

        let Some(alert) = alert else {
            continue;
        };

        // Log the alert (the EvalAlertSink seam can be threaded through here in
        // a future PR; for now eprintln! is the honest production path — identical
        // to how the eval worker and K-L3 alarm path work today).
        eprintln!(
            "[tdw] ALERT: knowledge watchlist fired \
             watch_id={} principal={} subject={:?} why={}",
            alert.watch_id,
            alert.principal_id,
            alert.subject.id(),
            alert.why,
        );

        // Advance last_alerted_as_of and increment alerts_fired under the lock.
        {
            if let Ok(mut guard) = store.lock() {
                guard.record_alert(&alert.watch_id, &alert.to_as_of);
            }
        }

        total_fired += 1;
    }

    // Update freshness cell.
    {
        let total_count = store.lock().map_or(0, |g| g.total_count());
        let cumulative = store.lock().map_or(0, |g| g.total_alerts_fired);
        if let Ok(mut f) = freshness.lock() {
            f.watch_count = total_count;
            f.last_check_ms = now_ms;
            f.total_alerts_fired = cumulative;
            f.note = if total_count == 0 {
                "no watchlists configured".to_string()
            } else {
                String::new()
            };
        }
    }

    total_fired
}

/// Check one entity watch by computing the K-X1 diff over its 2-hop
/// neighborhood between `last_alerted_as_of` and `now_as_of`.
///
/// Returns `Some(alert)` if the configured change types match changes found;
/// `None` otherwise (no alert needed).
async fn check_entity_watch(
    watch: &WatchEntry,
    entity_id: &str,
    graph: &Arc<dyn GraphEngine>,
    now_as_of: &str,
) -> Option<WatchlistAlert> {
    let from_ts = date_to_timestamp(&watch.last_alerted_as_of);
    let to_ts = date_to_timestamp(now_as_of);

    // Reuse K-X1 scoped-edge collection: 2-hop neighborhood.
    let raw_edges = collect_scoped_edges(graph, entity_id).await.ok()?;

    let limit = WATCHLIST_CHANGE_ITEM_LIMIT;
    let delta = compute_edge_delta(&raw_edges, &from_ts, &to_ts, limit);

    // Classify: which subscribed change types actually fired?
    let mut fired: Vec<String> = Vec::new();
    let mut new_fact_edges: Vec<Value> = Vec::new();
    let mut invalidated_edges: Vec<Value> = Vec::new();

    if watch.change_types.new_facts && delta.added_count > 0 {
        fired.push("new_facts".to_string());
        new_fact_edges = delta.added_items;
    }
    if (watch.change_types.invalidations || watch.change_types.contradictions)
        && delta.invalidated_count > 0
    {
        if watch.change_types.invalidations {
            fired.push("invalidations".to_string());
        }
        // Contradictions are surfaced via the invalidation bucket (K-M4 closes
        // edges whose valid_to is set in the window).
        if watch.change_types.contradictions
            && delta.invalidated_count > 0
            && !fired.contains(&"contradictions".to_string())
        {
            fired.push("contradictions".to_string());
        }
        invalidated_edges = delta.invalidated_items;
    }

    if fired.is_empty() {
        return None;
    }

    let why = format!(
        "entity {:?} neighborhood changed: {} new fact(s), {} invalidation(s) \
         between {} and {}",
        entity_id, delta.added_count, delta.invalidated_count, watch.last_alerted_as_of, now_as_of,
    );

    Some(WatchlistAlert {
        watch_id: watch.watch_id.clone(),
        principal_id: watch.principal_id.clone(),
        subject: watch.subject.clone(),
        from_as_of: watch.last_alerted_as_of.clone(),
        to_as_of: now_as_of.to_string(),
        fired_change_types: fired,
        new_fact_edges,
        invalidated_edges,
        why,
    })
}

/// Check one tag watch: compare `active_tags` at `from` vs `to` for all
/// entities holding this tag.
///
/// Currently this is a global tag-delta approach using the tag engine's own
/// comparison semantics.  No alert is fired if the tag engine's `entities_with_tag`
/// returns the same set at both time points.
async fn check_tag_watch(
    watch: &WatchEntry,
    tag_id: &str,
    tags: &Arc<dyn TagEngine>,
    now_as_of: &str,
) -> Option<WatchlistAlert> {
    let from_date = &watch.last_alerted_as_of;
    let to_date = now_as_of;

    let from_entities: BTreeSet<String> = tags
        .entities_with_tag(tag_id, from_date)
        .await
        .ok()?
        .into_iter()
        .collect();

    let to_entities: BTreeSet<String> = tags
        .entities_with_tag(tag_id, to_date)
        .await
        .ok()?
        .into_iter()
        .collect();

    let gained: Vec<String> = to_entities.difference(&from_entities).cloned().collect();
    let lost: Vec<String> = from_entities.difference(&to_entities).cloned().collect();

    if gained.is_empty() && lost.is_empty() {
        return None;
    }

    let mut fired: Vec<String> = Vec::new();
    let mut new_fact_edges: Vec<Value> = Vec::new();
    let mut invalidated_edges: Vec<Value> = Vec::new();

    if watch.change_types.new_facts && !gained.is_empty() {
        fired.push("new_facts".to_string());
        new_fact_edges = gained
            .iter()
            .take(WATCHLIST_CHANGE_ITEM_LIMIT)
            .map(|eid| json!({ "entity_id": eid, "tag_gained": tag_id }))
            .collect();
    }
    if watch.change_types.invalidations && !lost.is_empty() {
        fired.push("invalidations".to_string());
        invalidated_edges = lost
            .iter()
            .take(WATCHLIST_CHANGE_ITEM_LIMIT)
            .map(|eid| json!({ "entity_id": eid, "tag_lost": tag_id }))
            .collect();
    }

    if fired.is_empty() {
        return None;
    }

    let why = format!(
        "tag {:?} membership changed: {} entities gained, {} lost \
         between {} and {}",
        tag_id,
        gained.len(),
        lost.len(),
        from_date,
        to_date,
    );

    Some(WatchlistAlert {
        watch_id: watch.watch_id.clone(),
        principal_id: watch.principal_id.clone(),
        subject: watch.subject.clone(),
        from_as_of: watch.last_alerted_as_of.clone(),
        to_as_of: to_date.to_string(),
        fired_change_types: fired,
        new_fact_edges,
        invalidated_edges,
        why,
    })
}

// ── K-X1 diff machinery reuse ─────────────────────────────────────────────────
// These are local copies of the same pure functions in `knowledge_explain_tools`
// (which are `pub(crate)` there and thus not importable here without refactoring).
// They are small, pure, and correct — duplication is deliberate to keep K-X5
// additive (NEW file, no modification to K-X1 internals).

struct EdgeDelta {
    added_count: usize,
    invalidated_count: usize,
    added_items: Vec<Value>,
    invalidated_items: Vec<Value>,
}

/// Collect edges reachable within 2 hops from `scope_id` (K-X1 scoped path).
async fn collect_scoped_edges(
    graph: &Arc<dyn GraphEngine>,
    scope_id: &str,
) -> Result<Vec<GraphEdge>, ToolFailure> {
    let filter = TraversalFilter {
        direction: Direction::Both,
        max_hops: 2,
        rels: None,
        kinds: None,
        as_of: None, // intentionally no as_of: apply active_at ourselves
    };
    let subgraph = graph
        .expand(&[scope_id.to_string()], &filter)
        .await
        .map_err(|e| execution(e.to_string()))?;
    Ok(subgraph.edges)
}

/// Compute the edge-level delta — the same predicate pair that K-X1 uses.
///
/// Returns edges that became valid in `(from_ts, to_ts]` and edges whose
/// validity window closed in that period.  The `active_at` predicate is the
/// only temporal gate — no re-implementation, no leakage risk.
fn compute_edge_delta(
    raw_edges: &[GraphEdge],
    from_ts: &str,
    to_ts: &str,
    limit: usize,
) -> EdgeDelta {
    let active_at_from =
        |e: &&GraphEdge| active_at(e.valid_from.as_deref(), e.valid_to.as_deref(), from_ts);
    let active_at_to =
        |e: &&GraphEdge| active_at(e.valid_from.as_deref(), e.valid_to.as_deref(), to_ts);

    let added_raw: Vec<&GraphEdge> = raw_edges
        .iter()
        .filter(|e| !active_at_from(e) && active_at_to(e))
        .collect();
    let added_count = added_raw.len();
    let added_items: Vec<Value> = added_raw
        .iter()
        .take(limit)
        .map(|e| json!({ "from": e.from, "rel": e.rel, "to": e.to, "valid_from": e.valid_from }))
        .collect();

    let invalidated_raw: Vec<&GraphEdge> = raw_edges
        .iter()
        .filter(|e| active_at_from(e) && !active_at_to(e))
        .collect();
    let invalidated_count = invalidated_raw.len();
    let invalidated_items: Vec<Value> = invalidated_raw
        .iter()
        .take(limit)
        .map(|e| json!({ "from": e.from, "rel": e.rel, "to": e.to, "valid_to": e.valid_to }))
        .collect();

    EdgeDelta {
        added_count,
        invalidated_count,
        added_items,
        invalidated_items,
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Derive the stable watch id for a `(principal_id, subject_id)` pair.
fn watch_id_for(principal_id: &str, subject_id: &str) -> String {
    let input = format!("{principal_id}:{subject_id}");
    format!("watch:{:016x}", fnv1a64(input.as_bytes()))
}

/// FNV-1a 64-bit hash (stable; same as `knowledge_finding_tools`).
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Parse the optional `change_types` array.  Defaults to all-true when absent.
fn parse_change_types(arguments: &Map<String, Value>) -> Result<WatchChangeTypes, ToolFailure> {
    let Some(Value::Array(items)) = arguments.get("change_types") else {
        return Ok(WatchChangeTypes::default());
    };
    if items.is_empty() {
        return Err(execution(
            "change_types must be non-empty when supplied; omit the field to default to all"
                .to_string(),
        ));
    }
    let mut types = WatchChangeTypes {
        new_facts: false,
        invalidations: false,
        contradictions: false,
    };
    for item in items {
        let token = item
            .as_str()
            .ok_or_else(|| execution("change_types must be an array of strings".to_string()))?;
        if !VALID_CHANGE_TYPES.contains(&token) {
            return Err(execution(format!(
                "unknown change_type {token:?}: must be one of {VALID_CHANGE_TYPES:?}"
            )));
        }
        match token {
            "new_facts" => types.new_facts = true,
            "invalidations" => types.invalidations = true,
            "contradictions" => types.contradictions = true,
            _ => {}
        }
    }
    Ok(types)
}

/// Validate a graph-id token (`[A-Za-z0-9:._-]+`).
fn validate_graph_id(value: &str, field: &str) -> Result<(), ToolFailure> {
    if value.is_empty()
        || value
            .chars()
            .any(|c| !c.is_ascii_alphanumeric() && !matches!(c, ':' | '.' | '_' | '-'))
    {
        return Err(execution(format!(
            "invalid {field} {value:?}: must match [A-Za-z0-9:._-]+"
        )));
    }
    Ok(())
}

/// Require the bound principal (user id, K-L5 host-bound).
fn require_principal(runtime: &KnowledgeRuntime) -> Result<&str, ToolFailure> {
    runtime.bound_user_id().ok_or_else(|| {
        execution("no user identity bound — watchlist tools require a bound user id".to_string())
    })
}

/// Require the graph engine.
fn require_graph(runtime: &KnowledgeRuntime) -> Result<&Arc<dyn GraphEngine>, ToolFailure> {
    runtime.graph().ok_or_else(|| {
        execution(
            "knowledge graph not attached — watchlist tools require a graph engine".to_string(),
        )
    })
}

fn require_str<'a>(arguments: &'a Map<String, Value>, name: &str) -> Result<&'a str, ToolFailure> {
    optional_str(arguments, name)
        .ok_or_else(|| execution(format!("missing required argument: {name}")))
}

fn optional_str<'a>(arguments: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    arguments.get(name).and_then(Value::as_str)
}

/// Convert epoch milliseconds to a `YYYY-MM-DD` date string (UTC).
fn ms_to_date(ms: i64) -> String {
    // Simple Julian-day arithmetic — same approach as other date utilities in
    // this codebase.  No chrono dependency needed for this simple conversion.
    let secs = ms.max(0) / 1_000;
    let days = secs / 86_400;
    // Julian Day Number arithmetic (days since 1970-01-01 → calendar date).
    let julian = days + 2_440_588; // Julian Day Number for 1970-01-01
    let adj = julian + 32_044;
    let cent = (4 * adj + 3) / 146_097;
    let rem = adj - (146_097 * cent) / 4;
    let dyear = (4 * rem + 3) / 1_461;
    let dmonth = rem - (1_461 * dyear) / 4;
    let midx = (5 * dmonth + 2) / 153;
    let day = dmonth - (153 * midx + 2) / 5 + 1;
    let month = midx + 3 - 12 * (midx / 10);
    let year = 100 * cent + dyear - 4_800 + midx / 10;
    format!("{year:04}-{month:02}-{day:02}")
}

/// Convert a `YYYY-MM-DD` date string to the graph timestamp form used by
/// `tdw_tags::date_to_timestamp`.  Delegates to the tags crate to stay
/// consistent with the rest of the codebase.
fn date_to_timestamp(date: &str) -> String {
    tdw_tags::date_to_timestamp(date)
}

const fn execution(message: String) -> ToolFailure {
    ToolFailure::Execution(message)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    // ── WatchlistStore ─────────────────────────────────────────────────────────

    #[test]
    fn store_upsert_and_count() {
        let mut store = WatchlistStore::new();
        let entry = make_entry("u1", "instrument:AAPL");
        store.upsert(entry.clone());
        assert_eq!(store.total_count(), 1);
        assert_eq!(store.count_for_principal("u1"), 1);
        assert_eq!(store.count_for_principal("u2"), 0);
    }

    #[test]
    fn store_remove_returns_true_when_found() {
        let mut store = WatchlistStore::new();
        let entry = make_entry("u1", "instrument:AAPL");
        let watch_id = entry.watch_id.clone();
        store.upsert(entry);
        assert!(store.remove(&watch_id));
        assert_eq!(store.total_count(), 0);
    }

    #[test]
    fn store_remove_returns_false_when_missing() {
        let mut store = WatchlistStore::new();
        assert!(!store.remove("watch:nonexistent"));
    }

    #[test]
    fn store_by_principal_sorted_by_creation() {
        let mut store = WatchlistStore::new();
        let mut e1 = make_entry("u1", "instrument:AAPL");
        e1.created_as_of = "2026-01-01".to_string();
        let mut e2 = make_entry("u1", "instrument:MSFT");
        e2.created_as_of = "2026-06-01".to_string();
        store.upsert(e2.clone()); // insert newer first
        store.upsert(e1.clone());
        let by_u1 = store.by_principal("u1");
        assert_eq!(by_u1.len(), 2);
        assert_eq!(by_u1[0].created_as_of, "2026-01-01"); // sorted older first
    }

    #[test]
    fn store_record_alert_updates_fields() {
        let mut store = WatchlistStore::new();
        let entry = make_entry("u1", "instrument:AAPL");
        let watch_id = entry.watch_id.clone();
        store.upsert(entry);
        store.record_alert(&watch_id, "2026-06-12");
        let updated = store.entries.get(&watch_id).expect("entry");
        assert_eq!(updated.last_alerted_as_of, "2026-06-12");
        assert_eq!(updated.alerts_fired, 1);
        assert_eq!(store.total_alerts_fired, 1);
    }

    #[test]
    fn store_all_active_excludes_empty_change_types() {
        let mut store = WatchlistStore::new();
        let mut e = make_entry("u1", "instrument:AAPL");
        e.change_types = WatchChangeTypes {
            new_facts: false,
            invalidations: false,
            contradictions: false,
        };
        store.upsert(e);
        assert!(store.all_active().is_empty());
    }

    // ── watch_id_for ───────────────────────────────────────────────────────────

    #[test]
    fn watch_id_is_deterministic() {
        let id1 = watch_id_for("analyst", "instrument:AAPL");
        let id2 = watch_id_for("analyst", "instrument:AAPL");
        assert_eq!(id1, id2);
        assert!(id1.starts_with("watch:"));
    }

    #[test]
    fn watch_id_differs_by_subject() {
        let id1 = watch_id_for("analyst", "instrument:AAPL");
        let id2 = watch_id_for("analyst", "instrument:MSFT");
        assert_ne!(id1, id2);
    }

    #[test]
    fn watch_id_differs_by_principal() {
        let id1 = watch_id_for("analyst", "instrument:AAPL");
        let id2 = watch_id_for("other", "instrument:AAPL");
        assert_ne!(id1, id2);
    }

    // ── ms_to_date ─────────────────────────────────────────────────────────────

    #[test]
    fn ms_to_date_epoch_is_1970_01_01() {
        assert_eq!(ms_to_date(0), "1970-01-01");
    }

    #[test]
    fn ms_to_date_known_timestamp() {
        // 2025-06-12 00:00:00 UTC = 1_749_686_400_000 ms
        let ms: i64 = 1_749_686_400_000;
        assert_eq!(ms_to_date(ms), "2025-06-12");
    }

    // ── parse_change_types ─────────────────────────────────────────────────────

    #[test]
    fn parse_change_types_defaults_to_all_when_absent() {
        let args: Map<String, Value> = Map::new();
        let ct = parse_change_types(&args).unwrap_or_else(|_| panic!("parse failed"));
        assert!(ct.new_facts && ct.invalidations && ct.contradictions);
    }

    #[test]
    fn parse_change_types_subset() {
        let mut args: Map<String, Value> = Map::new();
        args.insert(
            "change_types".to_string(),
            json!(["new_facts", "contradictions"]),
        );
        let ct = parse_change_types(&args).unwrap_or_else(|_| panic!("parse failed"));
        assert!(ct.new_facts);
        assert!(!ct.invalidations);
        assert!(ct.contradictions);
    }

    #[test]
    fn parse_change_types_rejects_unknown_token() {
        let mut args: Map<String, Value> = Map::new();
        args.insert("change_types".to_string(), json!(["invented_type"]));
        assert!(parse_change_types(&args).is_err());
    }

    #[test]
    fn parse_change_types_rejects_empty_array() {
        let mut args: Map<String, Value> = Map::new();
        args.insert("change_types".to_string(), json!([]));
        assert!(parse_change_types(&args).is_err());
    }

    // ── validate_graph_id ──────────────────────────────────────────────────────

    #[test]
    fn validate_graph_id_accepts_valid_ids() {
        assert!(validate_graph_id("instrument:AAPL", "entity_id").is_ok());
        assert!(validate_graph_id("sector:tech", "tag_id").is_ok());
        assert!(validate_graph_id("a-b.c_d:e", "entity_id").is_ok());
    }

    #[test]
    fn validate_graph_id_rejects_empty() {
        assert!(validate_graph_id("", "entity_id").is_err());
    }

    #[test]
    fn validate_graph_id_rejects_spaces() {
        assert!(validate_graph_id("bad id", "entity_id").is_err());
    }

    // ── compute_edge_delta ─────────────────────────────────────────────────────

    #[test]
    fn compute_edge_delta_detects_added_edge() {
        use tdw_core::{GraphEdge, Provenance};

        // An edge with valid_from in the window: not active at T0, active at T1.
        let edge = GraphEdge {
            from: "a:1".to_string(),
            to: "b:2".to_string(),
            rel: "knows".to_string(),
            props: Value::Null,
            provenance: Provenance::System {
                detail: "test".to_string(),
            },
            valid_from: Some("2026-06-01T00:00:00Z".to_string()),
            valid_to: None,
        };
        let edges = vec![edge];
        let delta = compute_edge_delta(&edges, "2026-05-01T00:00:00Z", "2026-06-12T00:00:00Z", 10);
        assert_eq!(delta.added_count, 1);
        assert_eq!(delta.invalidated_count, 0);
    }

    #[test]
    fn compute_edge_delta_detects_invalidated_edge() {
        use tdw_core::{GraphEdge, Provenance};

        // An edge whose valid_to falls in the window.
        let edge = GraphEdge {
            from: "a:1".to_string(),
            to: "b:2".to_string(),
            rel: "knows".to_string(),
            props: Value::Null,
            provenance: Provenance::System {
                detail: "test".to_string(),
            },
            valid_from: Some("2026-01-01T00:00:00Z".to_string()),
            valid_to: Some("2026-06-05T00:00:00Z".to_string()),
        };
        let edges = vec![edge];
        let delta = compute_edge_delta(&edges, "2026-05-01T00:00:00Z", "2026-06-12T00:00:00Z", 10);
        assert_eq!(delta.added_count, 0);
        assert_eq!(delta.invalidated_count, 1);
    }

    #[test]
    fn compute_edge_delta_no_changes_returns_empty() {
        use tdw_core::{GraphEdge, Provenance};

        // An eternally-active edge (no valid_from/to) is active at both points.
        let edge = GraphEdge {
            from: "a:1".to_string(),
            to: "b:2".to_string(),
            rel: "knows".to_string(),
            props: Value::Null,
            provenance: Provenance::System {
                detail: "test".to_string(),
            },
            valid_from: None,
            valid_to: None,
        };
        let edges = vec![edge];
        let delta = compute_edge_delta(&edges, "2026-05-01T00:00:00Z", "2026-06-12T00:00:00Z", 10);
        assert_eq!(delta.added_count, 0);
        assert_eq!(delta.invalidated_count, 0);
    }

    // ── tick_watchlist_check ───────────────────────────────────────────────────

    #[tokio::test]
    async fn tick_with_no_watches_fires_nothing() {
        use std::sync::Arc;
        use tdw_storage_graph::InMemoryGraphEngine;
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        let store = Arc::new(Mutex::new(WatchlistStore::new()));
        let freshness = Arc::new(Mutex::new(WatchlistFreshness::default()));
        let now_ms = 1_749_686_400_000_i64; // 2025-06-12
        let fired = tick_watchlist_check(&store, &graph, None, now_ms, &freshness).await;
        assert_eq!(fired, 0);
        let f = freshness.lock().expect("freshness");
        assert_eq!(f.watch_count, 0);
        assert_eq!(f.last_check_ms, now_ms);
    }

    #[tokio::test]
    async fn tick_watched_entity_with_new_edge_fires_one_alert() {
        use std::sync::Arc;
        use tdw_core::{GraphEdge, GraphNode, Provenance};
        use tdw_storage_graph::InMemoryGraphEngine;
        use tdw_taxonomy::EntityKind;

        // Build a graph with entity instrument:AAPL and add an edge that appears
        // after the watch's last_alerted_as_of.
        let graph_engine = InMemoryGraphEngine::default();
        let graph: Arc<dyn GraphEngine> = Arc::new(graph_engine);

        // Seed two entity nodes (self-loops are excluded by the graph engine's
        // expand implementation — use a real edge between two distinct nodes).
        graph
            .upsert_nodes(vec![
                GraphNode {
                    id: "instrument:AAPL".to_string(),
                    kind: EntityKind::Instrument,
                    label: "Apple".to_string(),
                    aliases: vec![],
                    props: Value::Null,
                    valid_from: None,
                    valid_to: None,
                },
                GraphNode {
                    id: "instrument:AAPL_ETF".to_string(),
                    kind: EntityKind::Instrument,
                    label: "Apple Inc.".to_string(),
                    aliases: vec![],
                    props: Value::Null,
                    valid_from: None,
                    valid_to: None,
                },
            ])
            .await
            .expect("upsert nodes");

        // Add an edge that is NEW relative to 2025-05-01 but active by 2025-06-12.
        graph
            .upsert_edges(vec![GraphEdge {
                from: "instrument:AAPL".to_string(),
                to: "instrument:AAPL_ETF".to_string(),
                rel: "issued_by".to_string(),
                props: Value::Null,
                provenance: Provenance::System {
                    detail: "test".to_string(),
                },
                valid_from: Some("2025-06-01T00:00:00Z".to_string()),
                valid_to: None,
            }])
            .await
            .expect("upsert edge");

        let store = Arc::new(Mutex::new(WatchlistStore::new()));
        let freshness = Arc::new(Mutex::new(WatchlistFreshness::default()));

        // Create a watch with last_alerted_as_of = 2025-05-01 (before the edge).
        let mut entry = make_entry("analyst", "instrument:AAPL");
        entry.last_alerted_as_of = "2025-05-01".to_string();
        {
            let mut s = store.lock().expect("store");
            s.upsert(entry);
        }

        // now_ms corresponds to 2025-06-12 (1_749_686_400_000 ms since Unix epoch).
        let now_ms = 1_749_686_400_000_i64;
        let fired = tick_watchlist_check(&store, &graph, None, now_ms, &freshness).await;

        // Exactly one alert should have fired.
        assert_eq!(fired, 1);

        // The store should have advanced last_alerted_as_of.
        {
            let s = store.lock().expect("store");
            let watch_id = watch_id_for("analyst", "instrument:AAPL");
            let entry = s.entries.get(&watch_id).expect("entry");
            assert_eq!(entry.last_alerted_as_of, "2025-06-12");
            assert_eq!(entry.alerts_fired, 1);
        }
    }

    #[tokio::test]
    async fn tick_reruns_do_not_re_fire() {
        use std::sync::Arc;
        use tdw_core::{GraphEdge, GraphNode, Provenance};
        use tdw_storage_graph::InMemoryGraphEngine;
        use tdw_taxonomy::EntityKind;

        let graph_engine = InMemoryGraphEngine::default();
        let graph: Arc<dyn GraphEngine> = Arc::new(graph_engine);
        graph
            .upsert_nodes(vec![
                GraphNode {
                    id: "instrument:AAPL".to_string(),
                    kind: EntityKind::Instrument,
                    label: "Apple".to_string(),
                    aliases: vec![],
                    props: Value::Null,
                    valid_from: None,
                    valid_to: None,
                },
                GraphNode {
                    id: "instrument:AAPL_ETF".to_string(),
                    kind: EntityKind::Instrument,
                    label: "Apple Inc.".to_string(),
                    aliases: vec![],
                    props: Value::Null,
                    valid_from: None,
                    valid_to: None,
                },
            ])
            .await
            .expect("upsert nodes");
        graph
            .upsert_edges(vec![GraphEdge {
                from: "instrument:AAPL".to_string(),
                to: "instrument:AAPL_ETF".to_string(),
                rel: "issued_by".to_string(),
                props: Value::Null,
                provenance: Provenance::System {
                    detail: "test".to_string(),
                },
                valid_from: Some("2025-06-01T00:00:00Z".to_string()),
                valid_to: None,
            }])
            .await
            .expect("upsert edge");

        let store = Arc::new(Mutex::new(WatchlistStore::new()));
        let freshness = Arc::new(Mutex::new(WatchlistFreshness::default()));

        let mut entry = make_entry("analyst", "instrument:AAPL");
        entry.last_alerted_as_of = "2025-05-01".to_string();
        {
            let mut s = store.lock().expect("store");
            s.upsert(entry);
        }

        let now_ms = 1_749_686_400_000_i64; // 2025-06-12
        // First tick: alert fires.
        let fired1 = tick_watchlist_check(&store, &graph, None, now_ms, &freshness).await;
        assert_eq!(fired1, 1);

        // Second tick with the same now_ms: last_alerted_as_of == now_as_of, so
        // the window is zero-length and no alert fires.
        let fired2 = tick_watchlist_check(&store, &graph, None, now_ms, &freshness).await;
        assert_eq!(fired2, 0, "re-run must not re-fire the same alert");
    }

    #[tokio::test]
    async fn tick_unwatched_entity_change_fires_nothing() {
        use std::sync::Arc;
        use tdw_core::{GraphEdge, GraphNode, Provenance};
        use tdw_storage_graph::InMemoryGraphEngine;
        use tdw_taxonomy::EntityKind;

        let graph_engine = InMemoryGraphEngine::default();
        let graph: Arc<dyn GraphEngine> = Arc::new(graph_engine);
        // Entity exists but is NOT in the watchlist.
        graph
            .upsert_nodes(vec![GraphNode {
                id: "instrument:MSFT".to_string(),
                kind: EntityKind::Instrument,
                label: "Microsoft".to_string(),
                aliases: vec![],
                props: Value::Null,
                valid_from: None,
                valid_to: None,
            }])
            .await
            .expect("upsert node");
        graph
            .upsert_edges(vec![GraphEdge {
                from: "instrument:MSFT".to_string(),
                to: "instrument:MSFT".to_string(),
                rel: "has_rating".to_string(),
                props: Value::Null,
                provenance: Provenance::System {
                    detail: "test".to_string(),
                },
                valid_from: Some("2025-06-01T00:00:00Z".to_string()),
                valid_to: None,
            }])
            .await
            .expect("upsert edge");

        // Watchlist watches AAPL only.
        let store = Arc::new(Mutex::new(WatchlistStore::new()));
        let freshness = Arc::new(Mutex::new(WatchlistFreshness::default()));
        let mut entry = make_entry("analyst", "instrument:AAPL");
        entry.last_alerted_as_of = "2025-05-01".to_string();
        {
            let mut s = store.lock().expect("store");
            s.upsert(entry);
        }

        let now_ms = 1_749_686_400_000_i64;
        let fired = tick_watchlist_check(&store, &graph, None, now_ms, &freshness).await;
        // MSFT changed but only AAPL is watched — and AAPL's neighborhood is empty.
        // No alert fires.
        assert_eq!(fired, 0, "unwatched entity change must not fire alert");
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_entry(principal_id: &str, subject_id: &str) -> WatchEntry {
        WatchEntry {
            watch_id: watch_id_for(principal_id, subject_id),
            principal_id: principal_id.to_string(),
            subject: WatchSubject::Entity {
                entity_id: subject_id.to_string(),
            },
            change_types: WatchChangeTypes::default(),
            created_as_of: "2025-06-12".to_string(),
            last_alerted_as_of: "2025-06-12".to_string(),
            alerts_fired: 0,
        }
    }
}
