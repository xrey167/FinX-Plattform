//! The running memory-consolidation loop: a `MemoryStore` with JSON5 persistence,
//! a deterministic apply step over the pure [`consolidation_plan`] planner, and a
//! periodic background scheduler.
//!
//! Phase B turns the pure planner in `tdw-agent::consolidate` into a *running*
//! process. The planner stays the single source of truth for the decision (what
//! to promote/expire); this module owns the **state**: it holds memories keyed by
//! `meta.base.name`, round-trips them to `*.json5` files so a tier change survives
//! a restart, and applies the planner's actions (Promote rewrites the tier +
//! `last_consolidated`; Expire removes the entry).
//!
//! Everything is deterministic by injecting `now` ([`consolidate_at`] /
//! [`age_days`] take an RFC 3339 string), so tests never read the wall clock. Only
//! the live [`spawn_consolidation_scheduler`] task reads `chrono::Utc::now()`.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::DateTime;
use tdw_agent::{
    ConsolidationAction, Memory, RegistryEntity, Retention, UsageHint, consolidation_plan,
    consolidation_plan_with_usage, entity_from_resource, load_resource,
};
use tdw_knowledge::runtime::ConsolidationFreshness;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Snapshot of a single feedback event used by the scheduler's usage-aware path.
/// Fields: (`agent_id`, used, `recorded_at`, `hit_ids`, `query_fingerprint`).
type FeedbackSnapshot = Vec<(String, bool, String, Vec<String>, String)>;

/// An error from loading or persisting a [`MemoryStore`].
#[derive(Debug)]
pub enum MemoryStoreError {
    /// A filesystem error while reading or writing a memory file or directory.
    Io {
        /// The path that failed.
        path: String,
        /// The underlying IO error.
        source: io::Error,
    },
    /// A `*.json5` file did not parse as a [`Memory`] resource.
    Parse {
        /// The path that failed to parse.
        path: String,
        /// A human-readable reason.
        detail: String,
    },
    /// A memory could not be serialized back to its resource form.
    Serialize(String),
    /// A memory `name` is not a safe filename stem, so it cannot be mapped to a
    /// backing file without risking a write/delete outside the store directory.
    InvalidName(String),
}

impl std::fmt::Display for MemoryStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => write!(formatter, "io error for {path}: {source}"),
            Self::Parse { path, detail } => write!(formatter, "parse error for {path}: {detail}"),
            Self::Serialize(detail) => write!(formatter, "serialize error: {detail}"),
            Self::InvalidName(name) => write!(formatter, "invalid memory name: {name}"),
        }
    }
}

impl std::error::Error for MemoryStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Whole days between `last_consolidated` and `now`, both RFC 3339, saturating at 0.
///
/// `last_consolidated == None` is treated as age 0 (a memory with no breadcrumb has
/// not aged). Unparseable timestamps also yield 0 — the conservative choice that
/// never spuriously expires or promotes a memory.
#[must_use]
pub fn age_days(last_consolidated: Option<&str>, now: &str) -> u32 {
    let Some(last) = last_consolidated else {
        return 0;
    };
    let (Ok(last), Ok(now)) = (
        DateTime::parse_from_rfc3339(last),
        DateTime::parse_from_rfc3339(now),
    ) else {
        return 0;
    };
    let delta = now.signed_duration_since(last);
    let days = delta.num_days();
    u32::try_from(days.max(0)).unwrap_or(u32::MAX)
}

/// One memory plus where it persists: the `*.json5` file backing it, if any.
#[derive(Clone, Debug)]
struct Entry {
    memory: Memory,
    /// The file this memory round-trips to; `None` for an in-memory-only store.
    path: Option<PathBuf>,
}

/// An in-memory map of [`Memory`] entities keyed by `meta.base.name`, optionally
/// backed by a directory of `*.json5` files.
///
/// With a backing dir (via [`MemoryStore::load_dir`]) every mutation round-trips
/// to disk so a tier change survives a restart; [`MemoryStore::new`] is purely
/// in-memory (persistence calls are no-ops).
#[derive(Clone, Debug, Default)]
pub struct MemoryStore {
    entries: BTreeMap<String, Entry>,
    /// The directory new memories are written into; `None` for in-memory-only.
    dir: Option<PathBuf>,
}

impl MemoryStore {
    /// An empty, in-memory-only store (no backing directory).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Load every `*.json5` file in `dir` (non-recursive) as a [`Memory`], keyed by
    /// `meta.base.name`, remembering both the source dir and each memory's file path
    /// so later mutations persist back to the same file.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryStoreError::Io`] on a filesystem error or
    /// [`MemoryStoreError::Parse`] if a file is not a valid `Memory` resource.
    pub fn load_dir(dir: &Path) -> Result<Self, MemoryStoreError> {
        let mut entries = BTreeMap::new();
        let read_dir = std::fs::read_dir(dir).map_err(|source| MemoryStoreError::Io {
            path: dir.display().to_string(),
            source,
        })?;
        for entry in read_dir {
            let entry = entry.map_err(|source| MemoryStoreError::Io {
                path: dir.display().to_string(),
                source,
            })?;
            let path = entry.path();
            if path.extension().and_then(std::ffi::OsStr::to_str) != Some("json5") {
                continue;
            }
            let text = std::fs::read_to_string(&path).map_err(|source| MemoryStoreError::Io {
                path: path.display().to_string(),
                source,
            })?;
            let memory = parse_memory(&text, &path)?;
            let name = memory.meta.base.name.clone();
            entries.insert(
                name,
                Entry {
                    memory,
                    path: Some(path),
                },
            );
        }
        Ok(Self {
            entries,
            dir: Some(dir.to_path_buf()),
        })
    }

    /// Insert or replace a memory by `meta.base.name`, stamping `now` as its
    /// `last_consolidated` anchor when none is set (so a fresh memory ages from
    /// insertion), then persisting it when the store has a backing dir.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryStoreError`] if the backing file cannot be written.
    pub fn upsert_at(&mut self, mut memory: Memory, now: &str) -> Result<(), MemoryStoreError> {
        if memory.last_consolidated.is_none() {
            memory.last_consolidated = Some(now.to_string());
        }
        let name = memory.meta.base.name.clone();
        // The name is the map key AND the backing-file stem (`{name}.json5`).
        // `EntityMeta` only length-checks it, so enforce a safe single segment
        // here before it can reach `file_path_for` — otherwise a name like
        // `../../evil` or `foo/bar` would write/delete outside the store dir.
        tdw_core::safe_path_segment(&name)
            .map_err(|_| MemoryStoreError::InvalidName(name.clone()))?;
        // Preserve a previously-known backing path; otherwise assign one in the dir.
        let path = self
            .entries
            .get(&name)
            .and_then(|entry| entry.path.clone())
            .or_else(|| self.dir.as_ref().map(|dir| file_path_for(dir, &name)));
        self.entries.insert(
            name.clone(),
            Entry {
                memory,
                path: path.clone(),
            },
        );
        if path.is_some() {
            self.persist(&name)?;
        }
        Ok(())
    }

    /// An iterator over the stored memories.
    pub fn memories(&self) -> impl Iterator<Item = &Memory> {
        self.entries.values().map(|entry| &entry.memory)
    }

    /// Borrow a memory by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Memory> {
        self.entries.get(name).map(|entry| &entry.memory)
    }

    /// The number of stored memories.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the store holds no memories.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Serialize a memory back to its `*.json5` file (strict JSON is valid JSON5).
    /// A no-op when the memory has no backing path (an in-memory-only store).
    ///
    /// # Errors
    ///
    /// Returns [`MemoryStoreError::Serialize`] if the memory cannot be serialized,
    /// or [`MemoryStoreError::Io`] if the file cannot be written.
    pub fn persist(&self, name: &str) -> Result<(), MemoryStoreError> {
        let Some(entry) = self.entries.get(name) else {
            return Ok(());
        };
        let Some(path) = entry.path.as_ref() else {
            return Ok(());
        };
        let resource = entry
            .memory
            .to_resource()
            .map_err(|error| MemoryStoreError::Serialize(error.to_string()))?;
        let text = serde_json::to_string_pretty(&resource)
            .map_err(|error| MemoryStoreError::Serialize(error.to_string()))?;
        std::fs::write(path, text).map_err(|source| MemoryStoreError::Io {
            path: path.display().to_string(),
            source,
        })
    }

    /// Drop a memory from the map and, when backed by a file, delete that file
    /// (used to apply [`ConsolidationAction::Expire`]). Removing an absent name or
    /// a missing file is a no-op.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryStoreError::Io`] if the backing file exists but cannot be
    /// removed.
    pub fn remove(&mut self, name: &str) -> Result<(), MemoryStoreError> {
        let Some(entry) = self.entries.remove(name) else {
            return Ok(());
        };
        if let Some(path) = entry.path
            && path.exists()
        {
            std::fs::remove_file(&path).map_err(|source| MemoryStoreError::Io {
                path: path.display().to_string(),
                source,
            })?;
        }
        Ok(())
    }

    /// Promote a memory to a new [`Retention`] tier, stamping `now` as its new
    /// `last_consolidated` breadcrumb, then persisting the change when the store
    /// has a backing file.
    ///
    /// Used by callers outside this module (e.g. `tdw-backend`'s
    /// `consolidate_now`) that need to apply a [`ConsolidationAction::Promote`]
    /// without accessing the private `entries` field directly.
    ///
    /// Promoting an absent name is a no-op (mirrors the inline pattern in
    /// [`consolidate_at`]).
    ///
    /// # Errors
    ///
    /// Returns [`MemoryStoreError`] if the backing file cannot be written.
    pub fn promote_at(
        &mut self,
        name: &str,
        to: Retention,
        now: &str,
    ) -> Result<(), MemoryStoreError> {
        if let Some(entry) = self.entries.get_mut(name) {
            entry.memory.retention = to;
            entry.memory.last_consolidated = Some(now.to_string());
        }
        self.persist(name)
    }
}

/// Apply consolidation to the store at the injected `now` (RFC 3339).
///
/// Builds `(memory, age_days)` pairs, calls the pure [`consolidation_plan`] planner, then
/// applies each action — `Promote` rewrites the memory's tier + `last_consolidated`
/// (and persists), `Expire` removes the memory. Returns the applied actions.
///
/// `now` is a parameter so the apply step is deterministic and unit-testable
/// without a wall clock; only [`spawn_consolidation_scheduler`] reads the real
/// clock. A persistence failure aborts the remaining actions and surfaces the
/// error so a tier change is never silently lost.
///
/// # Errors
///
/// Returns [`MemoryStoreError`] if persisting a promotion or deleting an expired
/// memory's file fails.
pub fn consolidate_at(
    store: &mut MemoryStore,
    now: &str,
) -> Result<Vec<ConsolidationAction>, MemoryStoreError> {
    // The planner borrows the memories immutably; collect its decisions first, then
    // apply them under a fresh mutable borrow.
    let aged: Vec<(&Memory, u32)> = store
        .memories()
        .map(|memory| {
            let age = age_days(memory.last_consolidated.as_deref(), now);
            (memory, age)
        })
        .collect();
    let actions = consolidation_plan(aged);

    for action in &actions {
        match action {
            ConsolidationAction::Promote { name, to, .. } => {
                if let Some(entry) = store.entries.get_mut(name) {
                    entry.memory.retention = *to;
                    entry.memory.last_consolidated = Some(now.to_string());
                }
                store.persist(name)?;
            }
            ConsolidationAction::Expire { name } => {
                store.remove(name)?;
            }
        }
    }
    Ok(actions)
}

/// Spawn the periodic consolidation scheduler — the running counterpart to [`consolidate_at`].
///
/// Mirrors `tdw_app_server::spawn_inmemory_relay`: each tick it reads the wall clock once,
/// locks the store, and consolidates; shutdown is cooperative via the [`CancellationToken`].
///
/// This is the **only** place the real clock is read; the apply step stays
/// deterministic. A consolidation error on a tick is logged and the loop
/// continues (a transient FS error must not kill the scheduler), and the apply
/// step is additionally guarded against a panic so one bad tick can never
/// *silently* end the scheduler task: under `panic = "unwind"` the panic is
/// caught, logged, and the loop continues (tokio's `Mutex` does not poison, so
/// the store stays usable); under release `panic = "abort"` a panic aborts the
/// whole process loudly. Either way the failure is never invisible.
#[must_use]
pub fn spawn_consolidation_scheduler(
    store: Arc<Mutex<MemoryStore>>,
    tick: Duration,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            {
                let now = chrono::Utc::now().to_rfc3339();
                let mut guard = store.lock().await;
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    consolidate_at(&mut guard, &now)
                }));
                match outcome {
                    Ok(Ok(_actions)) => {}
                    Ok(Err(error)) => {
                        eprintln!("tdw-agent-store: consolidation tick error: {error}");
                    }
                    Err(_panic) => {
                        eprintln!(
                            "tdw-agent-store: consolidation tick panicked; the scheduler continues"
                        );
                    }
                }
            }
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(tick) => {}
            }
        }
    })
}

/// Spawn the periodic consolidation scheduler with retrieval-feedback awareness (K-L2).
///
/// This is the **production replacement** for [`spawn_consolidation_scheduler`]:
/// instead of calling the bare [`consolidate_at`] (which ignores usage data), each
/// tick drains the [`crate::RetrievalFeedbackStore`] and calls the usage-aware
/// planner ([`consolidation_plan_with_usage`]) so recency credit is applied before
/// any tier change or expiration. The resulting plan is applied to the store **and
/// persisted to `*.json5`** — tier changes survive a restart.
///
/// After every tick the [`ConsolidationFreshness`] cell is updated so
/// `tdw.kg.status` reflects the last tick time and applied counts.
///
/// Behaviour is **byte-for-byte identical to [`spawn_consolidation_scheduler`]**
/// when the feedback store is empty — the B10 regression contract is preserved.
///
/// Apply one consolidation tick with optional feedback events.
///
/// When `events` is `Some`, the usage-aware path computes recency credit and
/// calls [`consolidation_plan_with_usage`]; when `None`, falls through to the
/// bare [`consolidate_at`] (B10 regression contract).
fn apply_feedback_tick(
    guard: &mut MemoryStore,
    events: Option<FeedbackSnapshot>,
    now: &str,
) -> Result<Vec<ConsolidationAction>, MemoryStoreError> {
    let Some(events) = events else {
        return consolidate_at(guard, now);
    };

    let references = |memory_name: &str, agent_id: &str, hit_ids: &[String]| -> bool {
        agent_id == memory_name || hit_ids.iter().any(|h| h == memory_name)
    };

    let aged: Vec<_> = guard
        .memories()
        .map(|memory| {
            let name = memory.meta.base.name.as_str();
            let raw_age = age_days(memory.last_consolidated.as_deref(), now);
            let last_used_at = events
                .iter()
                .filter(|(agent_id, used, _, hit_ids, _)| {
                    *used && references(name, agent_id, hit_ids)
                })
                .map(|(_, _, recorded_at, _, _)| recorded_at.as_str())
                .max();
            let recency_credit_days = last_used_at.map_or(0, |t| {
                let days_since = age_days(Some(t), now);
                raw_age.saturating_sub(days_since)
            });
            let mut seen_fps = std::collections::HashSet::new();
            let use_count = u32::try_from(
                events
                    .iter()
                    .filter(|(agent_id, used, _, hit_ids, fp)| {
                        *used && references(name, agent_id, hit_ids) && seen_fps.insert(fp.as_str())
                    })
                    .count(),
            )
            .unwrap_or(u32::MAX);
            (
                memory,
                raw_age,
                UsageHint {
                    recency_credit_days,
                    use_count,
                },
            )
        })
        .collect();

    let actions = consolidation_plan_with_usage(aged);
    for action in &actions {
        match action {
            ConsolidationAction::Promote { name, to, .. } => {
                if let Some(entry) = guard.entries.get_mut(name) {
                    entry.memory.retention = *to;
                    entry.memory.last_consolidated = Some(now.to_string());
                }
                guard.persist(name)?;
            }
            ConsolidationAction::Expire { name } => {
                guard.remove(name)?;
            }
        }
    }
    Ok(actions)
}

/// # Panic safety
///
/// The apply step is wrapped in `catch_unwind` (same as the bare scheduler): a
/// panic on one tick is logged and the loop continues; the freshness cell is
/// written to `Error` so the failure is never invisible.
#[must_use]
pub fn spawn_consolidation_scheduler_with_feedback(
    store: Arc<Mutex<MemoryStore>>,
    feedback: Arc<Mutex<crate::RetrievalFeedbackStore>>,
    freshness: Arc<Mutex<ConsolidationFreshness>>,
    tick: Duration,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let now = chrono::Utc::now().to_rfc3339();

            // Snapshot feedback events (lock held only for this, never across store lock).
            let feedback_snapshot: Option<FeedbackSnapshot> = {
                let fb = feedback.lock().await;
                if fb.is_empty() {
                    None
                } else {
                    Some(
                        fb.events()
                            .map(|e| {
                                (
                                    e.agent_id.clone(),
                                    e.used,
                                    e.recorded_at.clone(),
                                    e.hit_ids.clone(),
                                    e.query_fingerprint.clone(),
                                )
                            })
                            .collect(),
                    )
                }
            };

            // Apply consolidation with usage-awareness.
            let (outcome, before_count, after_count) = {
                let mut guard = store.lock().await;
                let before_count = guard.len();
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    apply_feedback_tick(&mut guard, feedback_snapshot, &now)
                }));
                let after_count = guard.len();
                drop(guard);
                (outcome, before_count, after_count)
            };

            // Update the freshness cell and log.
            match outcome {
                Ok(Ok(actions)) => {
                    let applied_count = actions.len();
                    eprintln!(
                        "tdw-agent-store: consolidation tick at {now}: \
                         {before_count} memories before, {after_count} after, \
                         {applied_count} actions applied"
                    );
                    let mut cell = freshness.lock().await;
                    *cell = ConsolidationFreshness::Ok {
                        last_tick_at: now,
                        before_count,
                        after_count,
                        applied_count,
                    };
                }
                Ok(Err(error)) => {
                    let msg = error.to_string();
                    eprintln!("tdw-agent-store: consolidation tick error at {now}: {msg}");
                    let mut cell = freshness.lock().await;
                    *cell = ConsolidationFreshness::Error {
                        last_tick_at: now,
                        error: msg,
                    };
                }
                Err(_panic) => {
                    eprintln!(
                        "tdw-agent-store: consolidation tick panicked at {now}; \
                         the scheduler continues"
                    );
                    let mut cell = freshness.lock().await;
                    *cell = ConsolidationFreshness::Error {
                        last_tick_at: now,
                        error: "tick panicked".to_string(),
                    };
                }
            }

            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(tick) => {}
            }
        }
    })
}

/// Parse a `*.json5` memory file into a [`Memory`], wrapping the source path into
/// the error so a bad file is identifiable.
fn parse_memory(text: &str, path: &Path) -> Result<Memory, MemoryStoreError> {
    let resource = load_resource(text).map_err(|error| MemoryStoreError::Parse {
        path: path.display().to_string(),
        detail: error.to_string(),
    })?;
    entity_from_resource::<Memory>(&resource).map_err(|error| MemoryStoreError::Parse {
        path: path.display().to_string(),
        detail: error.to_string(),
    })
}

/// Build the backing file path for a memory `name` inside `dir`. A `Memory` keys
/// on `meta.base.name`, which is an agent identifier (alphanumeric plus `.`/`_`/`-`)
/// so it is filesystem-safe as a stem.
fn file_path_for(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.json5"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tdw_agent::{
        Adaptivity, DataFacets, EntityMeta, Materialization, Origin, Plane, Retention, Source, Tier,
    };

    /// A unique temp directory under the OS temp dir, created fresh per test.
    fn unique_temp_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("tdw-memstore-{tag}-{pid}-{n}"));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn memory(name: &str, retention: Retention, last_consolidated: Option<&str>) -> Memory {
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

    #[test]
    fn age_days_counts_whole_days_and_saturates_at_zero() {
        assert_eq!(
            age_days(Some("2026-05-01T00:00:00Z"), "2026-05-03T00:00:00Z"),
            2
        );
        // Less than a full day rounds down to 0.
        assert_eq!(
            age_days(Some("2026-05-01T00:00:00Z"), "2026-05-01T23:00:00Z"),
            0
        );
        // now before last → saturates at 0, never negative.
        assert_eq!(
            age_days(Some("2026-05-05T00:00:00Z"), "2026-05-01T00:00:00Z"),
            0
        );
        // None last_consolidated → age 0.
        assert_eq!(age_days(None, "2026-05-01T00:00:00Z"), 0);
        // Unparseable → conservative 0.
        assert_eq!(age_days(Some("not-a-date"), "2026-05-01T00:00:00Z"), 0);
    }

    #[test]
    fn consolidate_at_applies_expire_promote_core_and_fresh() {
        let now = "2026-05-10T00:00:00Z";
        let mut store = MemoryStore::new();
        // Working: ttl 0 → expires on the first call regardless of age.
        store
            .upsert_at(memory("buf", Retention::Working, Some(now)), now)
            .expect("upsert working");
        // ShortTerm aged 2 days (ttl 1) → promotes to MidTerm.
        store
            .upsert_at(
                memory("note", Retention::ShortTerm, Some("2026-05-08T00:00:00Z")),
                now,
            )
            .expect("upsert short");
        // Core → never changes.
        store
            .upsert_at(
                memory("identity", Retention::Core, Some("2020-01-01T00:00:00Z")),
                now,
            )
            .expect("upsert core");
        // Fresh ShortTerm whose last_consolidated == now → left alone (age 0 < ttl 1).
        store
            .upsert_at(memory("fresh", Retention::ShortTerm, Some(now)), now)
            .expect("upsert fresh");

        let actions = consolidate_at(&mut store, now).expect("consolidate");

        // buf expired (gone), note promoted, identity untouched, fresh untouched.
        assert!(store.get("buf").is_none(), "working buffer should expire");
        assert_eq!(
            store.get("note").map(|m| m.retention),
            Some(Retention::MidTerm),
            "aged short-term promotes to mid-term"
        );
        assert_eq!(
            store.get("note").and_then(|m| m.last_consolidated.clone()),
            Some(now.to_string()),
            "promotion stamps last_consolidated to now"
        );
        assert_eq!(
            store.get("identity").map(|m| m.retention),
            Some(Retention::Core),
            "core never changes"
        );
        assert_eq!(
            store.get("fresh").map(|m| m.retention),
            Some(Retention::ShortTerm),
            "a fresh memory is left alone"
        );

        // Exactly the expire + promote actions were returned (order: store iter order
        // is by name — buf, fresh, identity, note).
        assert!(actions.contains(&ConsolidationAction::Expire {
            name: "buf".to_string()
        }));
        assert!(actions.contains(&ConsolidationAction::Promote {
            name: "note".to_string(),
            from: Retention::ShortTerm,
            to: Retention::MidTerm,
        }));
        assert_eq!(actions.len(), 2, "only buf + note produce actions");
    }

    #[test]
    fn upsert_stamps_last_consolidated_when_absent() {
        let now = "2026-05-10T00:00:00Z";
        let mut store = MemoryStore::new();
        store
            .upsert_at(memory("x", Retention::ShortTerm, None), now)
            .expect("upsert");
        assert_eq!(
            store.get("x").and_then(|m| m.last_consolidated.clone()),
            Some(now.to_string()),
            "a fresh memory is stamped with the insertion `now`"
        );
    }

    #[test]
    fn json5_round_trip_persists_tier_across_reload() {
        let dir = unique_temp_dir("roundtrip");
        let now = "2026-05-10T00:00:00Z";

        // Seed a ShortTerm memory aged past its TTL, persisted to the dir.
        {
            let mut store = MemoryStore::load_dir(&dir).expect("load empty dir");
            store
                .upsert_at(
                    memory("note", Retention::ShortTerm, Some("2026-05-08T00:00:00Z")),
                    now,
                )
                .expect("upsert");
            // Mutate via the apply step: ShortTerm aged 2 days → MidTerm.
            let actions = consolidate_at(&mut store, now).expect("consolidate");
            assert_eq!(actions.len(), 1, "exactly one promotion");
        }

        // Fresh store, SAME dir — the persisted tier must survive the "restart".
        let reloaded = MemoryStore::load_dir(&dir).expect("reload dir");
        assert_eq!(
            reloaded.get("note").map(|m| m.retention),
            Some(Retention::MidTerm),
            "the promoted tier survives a reload from disk"
        );
        assert_eq!(
            reloaded
                .get("note")
                .and_then(|m| m.last_consolidated.clone()),
            Some(now.to_string()),
            "the consolidation breadcrumb survives a reload"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn upsert_at_rejects_path_traversal_names() {
        let dir = unique_temp_dir("traversal");
        let now = "2026-05-10T00:00:00Z";
        let mut store = MemoryStore::load_dir(&dir).expect("load empty dir");

        // A name with traversal/separator parts would otherwise resolve to a
        // path outside `dir` for fs::write / fs::remove_file. Each must be
        // rejected with InvalidName before any path is derived or stored.
        for bad in ["../../evil", "foo/bar", "..", "/abs"] {
            let result = store.upsert_at(memory(bad, Retention::Working, Some(now)), now);
            assert!(
                matches!(result, Err(MemoryStoreError::InvalidName(_))),
                "name {bad:?} must be rejected, got {result:?}"
            );
            // The rejected name must not have been inserted into the store.
            assert!(store.get(bad).is_none(), "{bad:?} must not be stored");
        }

        // A normal single-segment name still works.
        store
            .upsert_at(memory("research.note", Retention::Working, Some(now)), now)
            .expect("valid name persists");
        assert!(store.get("research.note").is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn expire_deletes_the_backing_file() {
        let dir = unique_temp_dir("expire");
        let now = "2026-05-10T00:00:00Z";
        {
            let mut store = MemoryStore::load_dir(&dir).expect("load dir");
            store
                .upsert_at(memory("buf", Retention::Working, Some(now)), now)
                .expect("upsert");
            consolidate_at(&mut store, now).expect("consolidate");
            assert!(store.get("buf").is_none(), "expired in-memory");
        }
        // The file is gone, so a fresh load sees no memory.
        let reloaded = MemoryStore::load_dir(&dir).expect("reload");
        assert!(
            reloaded.get("buf").is_none(),
            "the expired memory's file is deleted"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn scheduler_starts_and_stops_cleanly() {
        let now = "2026-05-10T00:00:00Z";
        let mut seed = MemoryStore::new();
        seed.upsert_at(memory("buf", Retention::Working, Some(now)), now)
            .expect("seed");
        let store = Arc::new(Mutex::new(seed));

        let cancel = CancellationToken::new();
        let handle =
            spawn_consolidation_scheduler(store.clone(), Duration::from_millis(5), cancel.clone());

        // The first tick (uses the real clock) expires the Working buffer.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            if store.lock().await.get("buf").is_none() {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            store.lock().await.get("buf").is_none(),
            "the live scheduler consolidates on its first tick"
        );

        // Cancellation stops the loop promptly and the task joins.
        cancel.cancel();
        let joined = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(joined.is_ok(), "scheduler must stop cleanly on cancel");
    }
}
