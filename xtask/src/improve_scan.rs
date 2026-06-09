//! `improve-scan`: deterministic improvement-backlog discovery for batch mode.
//!
//! Scans four debt buckets (lint-debt, test-gaps, provider-wiring, hygiene) and
//! writes a ranked, id-stable backlog to `.batch/backlog.json`. The backlog is
//! SINGLE-WRITER: only this command (via `/batch discover`) mutates it; batch
//! execution worktrees consume it read-only and append per-batch ledger files
//! under `.batch/ledger/`, whose outcomes this command folds back on the next
//! run. See `docs/batch-improvement.md` for the full contract.
//!
//! Design rules (consensus plan, 2026-06-06):
//! - Machines enumerate, agents judge: this command only measures; it never
//!   fixes anything and is NOT part of the quality-gate manifest.
//! - Drift-tolerant parsing: unrecognized clippy/metadata JSON lines are
//!   skipped (counted, warned), never fatal — a parse failure degrades one
//!   bucket, not the scan (same pattern as `summarize_outcomes` in `main.rs`).
//! - Scoped idempotency: same toolchain + same debt ⇒ re-run writes nothing
//!   (the `generatedAt` stamp is preserved when items are unchanged).
//! - Merge semantics are evidence-driven: `blocked`/`in-review` survive
//!   re-scans; `done`/`resolved` items whose evidence reappears are reopened to
//!   `pending` with a `reopenedFrom` marker; vanished evidence marks an item
//!   `resolved`; `resolved` items are pruned one cycle later (ledger files are
//!   the permanent history). Buckets that were skipped this run never resolve
//!   their items (no evidence ≠ vanished evidence).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const BACKLOG_PATH: &str = ".batch/backlog.json";
const LEDGER_DIR: &str = ".batch/ledger";
const SERVICE_API_MANIFEST: &str = "crates/tdw-service-api/Cargo.toml";

/// Provider crates excluded from the wiring bucket entirely (test doubles).
const PROVIDER_EXCLUDE: &[&str] = &["ws-mock"];

/// Deterministic bucket ordering for the ranked backlog.
const BUCKET_ORDER: &[&str] = &["provider-wiring", "lint-debt", "test-gaps", "hygiene"];

/// One backlog item. `id` is the stable merge key (derivation scheme:
/// `lint:<lint-code>`, `test-gap:<crate>`, `provider:<name>`,
/// `hygiene:<topic>`), documented in `docs/batch-improvement.md`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Item {
    id: String,
    bucket: String,
    crates: Vec<String>,
    title: String,
    evidence: String,
    effort: String,
    status: String,
    /// Count used for ranking inside a bucket (higher = earlier). Not part of
    /// the evidence text so re-ranking never breaks id stability.
    count: u64,
    /// Set when reappearing evidence reopened a `done`/`resolved` item.
    reopened_from: Option<String>,
}

impl Item {
    fn new(
        id: &str,
        bucket: &str,
        crates: Vec<String>,
        title: &str,
        evidence: String,
        effort: &str,
        count: u64,
    ) -> Self {
        Self {
            id: id.to_string(),
            bucket: bucket.to_string(),
            crates,
            title: title.to_string(),
            evidence,
            effort: effort.to_string(),
            status: "pending".to_string(),
            count,
            reopened_from: None,
        }
    }

    fn to_value(&self) -> serde_json::Value {
        let mut value = serde_json::json!({
            "id": self.id,
            "bucket": self.bucket,
            "crates": self.crates,
            "title": self.title,
            "evidence": self.evidence,
            "effort": self.effort,
            "status": self.status,
            "count": self.count,
        });
        if let Some(from) = &self.reopened_from {
            value["reopenedFrom"] = serde_json::Value::String(from.clone());
        }
        value
    }

    /// Drift-tolerant: missing/odd fields fall back to defaults instead of
    /// failing, so a hand-edited backlog cannot brick the scan.
    fn from_value(value: &serde_json::Value) -> Option<Self> {
        let id = value.get("id")?.as_str()?.to_string();
        let string_field = |key: &str| {
            value
                .get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let crates = value
            .get("crates")
            .and_then(serde_json::Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| entry.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            id,
            bucket: string_field("bucket"),
            crates,
            title: string_field("title"),
            evidence: string_field("evidence"),
            effort: string_field("effort"),
            status: {
                let status = string_field("status");
                if status.is_empty() {
                    "pending".to_string()
                } else {
                    status
                }
            },
            count: value
                .get("count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            reopened_from: value
                .get("reopenedFrom")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
        })
    }
}

/// Entry point for `cargo run -p xtask -- improve-scan`.
pub fn improve_scan() -> Result<(), String> {
    let mut fresh = Vec::new();
    let mut scanned: BTreeSet<String> = BTreeSet::new();

    println!("improve-scan: [1/4] provider-wiring (set-difference + normalization)");
    match scan_provider_wiring() {
        Ok(mut items) => {
            scanned.insert("provider-wiring".to_string());
            fresh.append(&mut items);
        }
        Err(error) => println!("improve-scan: provider-wiring skipped: {error}"),
    }

    println!("improve-scan: [2/4] lint-debt (clippy pedantic/nursery, JSON)");
    match scan_lint_debt() {
        Ok(mut items) => {
            scanned.insert("lint-debt".to_string());
            fresh.append(&mut items);
        }
        Err(error) => println!("improve-scan: lint-debt skipped: {error}"),
    }

    println!("improve-scan: [3/4] test-gaps (crates without tests, reverse-dep ranked)");
    match scan_test_gaps() {
        Ok(mut items) => {
            scanned.insert("test-gaps".to_string());
            fresh.append(&mut items);
        }
        Err(error) => println!("improve-scan: test-gaps skipped: {error}"),
    }

    println!("improve-scan: [4/4] hygiene (cargo-deny advisories, TODO/FIXME)");
    match scan_hygiene() {
        Ok(mut items) => {
            scanned.insert("hygiene".to_string());
            fresh.append(&mut items);
        }
        Err(error) => println!("improve-scan: hygiene skipped: {error}"),
    }

    let existing_content = fs::read_to_string(BACKLOG_PATH).ok();
    let existing_items = existing_content
        .as_deref()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(content).ok())
        .and_then(|value| {
            value
                .get("items")
                .and_then(serde_json::Value::as_array)
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(Item::from_value)
                        .collect::<Vec<_>>()
                })
        })
        .unwrap_or_default();

    let ledger_outcomes = read_ledger_outcomes(Path::new(LEDGER_DIR));
    let merged = merge_backlog(&existing_items, &ledger_outcomes, fresh, &scanned);

    write_backlog(&merged, &scanned, existing_content.as_deref())
}

// ---------------------------------------------------------------------------
// provider-wiring
// ---------------------------------------------------------------------------

fn scan_provider_wiring() -> Result<Vec<Item>, String> {
    let mut provider_dirs = Vec::new();
    for entry in fs::read_dir("crates").map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.path().is_dir()
            && let Some(provider) = name.strip_prefix("tdw-provider-")
        {
            provider_dirs.push(provider.to_string());
        }
    }
    provider_dirs.sort();

    let manifest = fs::read_to_string(SERVICE_API_MANIFEST).map_err(|error| error.to_string())?;
    let feature_keys = provider_feature_keys(&manifest);

    let mut has_http = BTreeMap::new();
    for provider in &provider_dirs {
        let path = PathBuf::from("crates")
            .join(format!("tdw-provider-{provider}"))
            .join("Cargo.toml");
        let content = fs::read_to_string(&path).unwrap_or_default();
        has_http.insert(provider.clone(), manifest_has_http_feature(&content));
    }

    Ok(classify_providers(&provider_dirs, &feature_keys, &has_http))
}

/// Extract `provider-*` feature KEY definitions (not list entries) from the
/// service-api manifest. A definition line is `provider-<name> = [...`; list
/// entries inside `all-http-providers` start with a quote and are ignored.
fn provider_feature_keys(manifest: &str) -> BTreeSet<String> {
    manifest
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if !trimmed.starts_with("provider-") {
                return None;
            }
            let (key, _) = trimmed.split_once('=')?;
            Some(key.trim().to_string())
        })
        .collect()
}

/// Whether a provider crate's own manifest defines an `http` feature.
fn manifest_has_http_feature(manifest: &str) -> bool {
    manifest.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == "http = []" || trimmed.starts_with("http = [")
    })
}

/// Normalization spec (consensus plan): a provider dir `<name>` is wired if
/// the key `provider-<name>` OR `provider-<name>-http` exists (worked example:
/// `binance` is wired via `provider-binance-http`). Unwired dirs with their own
/// `http` feature are `unwired-http` (the actionable target, e.g. `yahoo`);
/// unwired dirs without one are `needs-design` (e.g. `fileset`, `ws`).
/// `ws-mock` (test double) is excluded entirely.
fn classify_providers(
    provider_dirs: &[String],
    feature_keys: &BTreeSet<String>,
    has_http: &BTreeMap<String, bool>,
) -> Vec<Item> {
    let mut items = Vec::new();
    for provider in provider_dirs {
        if PROVIDER_EXCLUDE.contains(&provider.as_str()) {
            continue;
        }
        let wired = feature_keys.contains(&format!("provider-{provider}"))
            || feature_keys.contains(&format!("provider-{provider}-http"));
        if wired {
            continue;
        }
        let crate_name = format!("tdw-provider-{provider}");
        if has_http.get(provider).copied().unwrap_or(false) {
            items.push(Item::new(
                &format!("provider:{provider}"),
                "provider-wiring",
                vec![crate_name.clone(), "tdw-service-api".to_string()],
                &format!("Wire {crate_name} live HTTP fetcher into default_registry()"),
                format!(
                    "{crate_name} has an `http` feature but no `provider-{provider}`(-http) key in {SERVICE_API_MANIFEST}"
                ),
                "M",
                1,
            ));
        } else {
            items.push(Item::new(
                &format!("provider:{provider}"),
                "provider-wiring",
                vec![crate_name.clone(), "tdw-service-api".to_string()],
                &format!("Design wiring for {crate_name} (no `http` feature)"),
                format!(
                    "{crate_name} has no `http` feature and no `provider-{provider}`(-http) key — needs a wiring design, not the standard fetcher pattern"
                ),
                "L",
                0,
            ));
            if let Some(item) = items.last_mut() {
                item.status = "needs-design".to_string();
            }
        }
    }
    items
}

// ---------------------------------------------------------------------------
// lint-debt
// ---------------------------------------------------------------------------

fn scan_lint_debt() -> Result<Vec<Item>, String> {
    let output = std::process::Command::new("cargo")
        .args([
            "clippy",
            "--workspace",
            "--all-targets",
            "--message-format=json",
            "--",
            "-W",
            "clippy::pedantic",
            "-W",
            "clippy::nursery",
        ])
        .output()
        .map_err(|error| format!("failed to spawn cargo clippy: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo clippy exited with {}; lint bucket degraded (compile error or toolchain issue)",
            output.status
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (counts, skipped) = parse_clippy_lines(stdout.lines());
    if skipped > 0 {
        println!("improve-scan: lint-debt tolerated {skipped} unrecognized clippy JSON lines");
    }
    Ok(lint_items(&counts))
}

/// Aggregate clippy warning counts per (lint code, crate). Drift-tolerant:
/// any line that is not valid JSON or lacks the expected fields is skipped and
/// counted, never fatal (the schema is owned by rustc/clippy, not this repo).
fn parse_clippy_lines<'a>(
    lines: impl Iterator<Item = &'a str>,
) -> (BTreeMap<(String, String), u64>, u64) {
    let mut counts: BTreeMap<(String, String), u64> = BTreeMap::new();
    let mut skipped = 0u64;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            skipped += 1;
            continue;
        };
        if parsed.get("reason").and_then(serde_json::Value::as_str) != Some("compiler-message") {
            continue;
        }
        let Some(message) = parsed.get("message") else {
            skipped += 1;
            continue;
        };
        if message.get("level").and_then(serde_json::Value::as_str) != Some("warning") {
            continue;
        }
        let Some(code) = message
            .get("code")
            .and_then(|code| code.get("code"))
            .and_then(serde_json::Value::as_str)
        else {
            continue; // e.g. the aggregate "N warnings emitted" message
        };
        if !code.starts_with("clippy::") {
            continue;
        }
        let crate_name = parsed
            .get("target")
            .and_then(|target| target.get("name"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("workspace")
            .to_string();
        *counts.entry((code.to_string(), crate_name)).or_insert(0) += 1;
    }
    (counts, skipped)
}

/// One item per lint family (id `lint:<code>`), spanning the crates it occurs
/// in — matching the proven batch size of one lint family per PR.
fn lint_items(counts: &BTreeMap<(String, String), u64>) -> Vec<Item> {
    let mut per_lint: BTreeMap<String, (BTreeSet<String>, u64)> = BTreeMap::new();
    for ((code, crate_name), count) in counts {
        let entry = per_lint.entry(code.clone()).or_default();
        entry.0.insert(crate_name.clone());
        entry.1 += count;
    }
    per_lint
        .into_iter()
        .map(|(code, (crates, total))| {
            let effort = if total < 10 {
                "S"
            } else if total < 50 {
                "M"
            } else {
                "L"
            };
            Item::new(
                &format!("lint:{code}"),
                "lint-debt",
                crates.iter().cloned().collect(),
                &format!("Resolve {code} ({total} warnings)"),
                format!("{total} warnings across {} crates", crates.len()),
                effort,
                total,
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// test-gaps
// ---------------------------------------------------------------------------

fn scan_test_gaps() -> Result<Vec<Item>, String> {
    let mut gaps = Vec::new();
    for entry in fs::read_dir("crates").map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if path.join("tests").is_dir() || tree_contains_test_attribute(&path.join("src")) {
            continue;
        }
        gaps.push(name);
    }
    gaps.sort();
    let reverse_deps = workspace_reverse_deps()?;
    Ok(test_gap_items(&gaps, &reverse_deps))
}

/// Whether any `.rs` file under `dir` contains a `#[test]`/`#[tokio::test]`.
fn tree_contains_test_attribute(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if tree_contains_test_attribute(&path) {
                return true;
            }
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs")
            && fs::read_to_string(&path).is_ok_and(|content| {
                content.contains("#[test]") || content.contains("#[tokio::test]")
            })
        {
            return true;
        }
    }
    false
}

/// Workspace-internal reverse-dependency counts via `cargo metadata --no-deps`.
/// Drift-tolerant: missing fields are skipped, never fatal.
fn workspace_reverse_deps() -> Result<BTreeMap<String, u64>, String> {
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|error| format!("failed to spawn cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(format!("cargo metadata exited with {}", output.status));
    }
    let parsed: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))
        .map_err(|error| error.to_string())?;
    Ok(reverse_deps_from_metadata(&parsed))
}

fn reverse_deps_from_metadata(metadata: &serde_json::Value) -> BTreeMap<String, u64> {
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let Some(packages) = metadata
        .get("packages")
        .and_then(serde_json::Value::as_array)
    else {
        return counts;
    };
    let members: BTreeSet<&str> = packages
        .iter()
        .filter_map(|package| package.get("name").and_then(serde_json::Value::as_str))
        .collect();
    for package in packages {
        let Some(dependencies) = package
            .get("dependencies")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for dependency in dependencies {
            if let Some(name) = dependency.get("name").and_then(serde_json::Value::as_str)
                && members.contains(name)
            {
                *counts.entry(name.to_string()).or_insert(0) += 1;
            }
        }
    }
    counts
}

fn test_gap_items(gaps: &[String], reverse_deps: &BTreeMap<String, u64>) -> Vec<Item> {
    gaps.iter()
        .map(|name| {
            let dependents = reverse_deps.get(name).copied().unwrap_or(0);
            let effort = if dependents > 5 { "M" } else { "S" };
            Item::new(
                &format!("test-gap:{name}"),
                "test-gaps",
                vec![name.clone()],
                &format!("Add test coverage for {name}"),
                format!(
                    "no tests/ dir and no #[test]/#[tokio::test] in src; {dependents} workspace crates depend on it"
                ),
                effort,
                dependents,
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// hygiene
// ---------------------------------------------------------------------------

fn scan_hygiene() -> Result<Vec<Item>, String> {
    let mut items = Vec::new();

    // cargo-deny advisories: degrade to skipped when the tool or its advisory
    // DB is unavailable (offline guarantee) — reuses the tool CI already runs.
    match std::process::Command::new("cargo")
        .args(["deny", "check", "advisories"])
        .output()
    {
        Ok(output) if !output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let advisories = stderr.matches("ID:").count().max(1) as u64;
            items.push(Item::new(
                "hygiene:advisories",
                "hygiene",
                Vec::new(),
                "Resolve cargo-deny advisory failures",
                format!("`cargo deny check advisories` failed ({advisories} advisory marker(s) in output)"),
                "M",
                advisories,
            ));
        }
        Ok(_) => {}
        Err(error) => {
            println!(
                "improve-scan: hygiene advisories check skipped (cargo-deny unavailable: {error})"
            );
        }
    }

    let todo_count = count_todo_markers(Path::new("crates"))?;
    if todo_count > 0 {
        items.push(Item::new(
            "hygiene:todo-markers",
            "hygiene",
            Vec::new(),
            &format!("Burn down TODO/FIXME markers ({todo_count})"),
            format!("{todo_count} TODO/FIXME lines under crates/"),
            "S",
            todo_count,
        ));
    }

    Ok(items)
}

fn count_todo_markers(dir: &Path) -> Result<u64, String> {
    let mut count = 0u64;
    if !dir.exists() {
        return Ok(0);
    }
    for entry in fs::read_dir(dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            count += count_todo_markers(&path)?;
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs")
            && let Ok(content) = fs::read_to_string(&path)
        {
            count += content
                .lines()
                .filter(|line| line.contains("TODO") || line.contains("FIXME"))
                .count() as u64;
        }
    }
    Ok(count)
}

// ---------------------------------------------------------------------------
// ledger fold-back
// ---------------------------------------------------------------------------

/// Read per-batch ledger files and extract `item id -> outcome` transitions.
/// Ledger files carry a minimal front-matter block:
///
/// ```text
/// ---
/// batch: batch-lint-debt-001
/// items: lint:clippy::too_many_lines, lint:clippy::missing_panics_doc
/// outcome: done
/// ---
/// ```
///
/// Only `done` and `blocked` outcomes are folded; anything else (or a missing
/// or malformed block) is ignored — drift-tolerant, never fatal.
fn read_ledger_outcomes(dir: &Path) -> BTreeMap<String, String> {
    let mut outcomes = BTreeMap::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return outcomes;
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("md"))
        .collect();
    paths.sort();
    for path in paths {
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        for (id, outcome) in parse_ledger_front_matter(&content) {
            outcomes.insert(id, outcome);
        }
    }
    outcomes
}

fn parse_ledger_front_matter(content: &str) -> Vec<(String, String)> {
    let mut in_block = false;
    let mut items: Vec<String> = Vec::new();
    let mut outcome: Option<String> = None;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "---" {
            if in_block {
                break;
            }
            in_block = true;
            continue;
        }
        if !in_block {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("items:") {
            items = value
                .split(',')
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect();
        } else if let Some(value) = trimmed.strip_prefix("outcome:") {
            let value = value.trim();
            if value == "done" || value == "blocked" {
                outcome = Some(value.to_string());
            }
        }
    }
    outcome.map_or_else(Vec::new, |outcome| {
        items.into_iter().map(|id| (id, outcome.clone())).collect()
    })
}

// ---------------------------------------------------------------------------
// merge (the engineered, fixture-tested core)
// ---------------------------------------------------------------------------

/// Id-stable, status-preserving, evidence-driven merge. See module docs for
/// the full rules. `scanned_buckets` guards the vanished-evidence transition:
/// a bucket that was skipped this run contributes no evidence, so its existing
/// items pass through unchanged instead of being wrongly `resolved`.
fn merge_backlog(
    existing: &[Item],
    ledger_outcomes: &BTreeMap<String, String>,
    fresh: Vec<Item>,
    scanned_buckets: &BTreeSet<String>,
) -> Vec<Item> {
    let mut prior: BTreeMap<String, Item> = existing
        .iter()
        .map(|item| (item.id.clone(), item.clone()))
        .collect();
    for (id, outcome) in ledger_outcomes {
        if let Some(item) = prior.get_mut(id) {
            item.status.clone_from(outcome);
        }
    }

    let mut merged = Vec::new();
    let mut seen = BTreeSet::new();
    for mut item in fresh {
        seen.insert(item.id.clone());
        if let Some(old) = prior.get(&item.id) {
            match old.status.as_str() {
                "blocked" | "in-review" | "needs-design" => {
                    item.status.clone_from(&old.status);
                    item.reopened_from.clone_from(&old.reopened_from);
                }
                "done" | "resolved" => {
                    item.status = "pending".to_string();
                    item.reopened_from = Some(old.status.clone());
                }
                _ => {
                    item.reopened_from.clone_from(&old.reopened_from);
                }
            }
        }
        merged.push(item);
    }

    for (id, old) in prior {
        if seen.contains(&id) {
            continue;
        }
        if !scanned_buckets.contains(&old.bucket) {
            merged.push(old); // bucket skipped this run: no evidence signal
            continue;
        }
        if old.status == "resolved" {
            continue; // pruned one cycle after resolution; ledger is history
        }
        let mut resolved = old;
        resolved.status = "resolved".to_string();
        resolved.reopened_from = None;
        merged.push(resolved);
    }

    merged.sort_by(|a, b| {
        bucket_rank(&a.bucket)
            .cmp(&bucket_rank(&b.bucket))
            .then(b.count.cmp(&a.count))
            .then(a.id.cmp(&b.id))
    });
    merged
}

fn bucket_rank(bucket: &str) -> usize {
    BUCKET_ORDER
        .iter()
        .position(|known| *known == bucket)
        .unwrap_or(BUCKET_ORDER.len())
}

// ---------------------------------------------------------------------------
// output
// ---------------------------------------------------------------------------

/// Write `.batch/backlog.json`. Scoped idempotency: when the merged items and
/// toolchain are unchanged versus the existing file, nothing is written and
/// the previous `generatedAt` stamp survives (same toolchain ⇒ no diff).
fn write_backlog(
    items: &[Item],
    scanned: &BTreeSet<String>,
    existing_content: Option<&str>,
) -> Result<(), String> {
    let toolchain = rustc_version();
    let items_value: Vec<serde_json::Value> = items.iter().map(Item::to_value).collect();
    let buckets: BTreeMap<&str, &str> = BUCKET_ORDER
        .iter()
        .map(|bucket| {
            (
                *bucket,
                if scanned.contains(*bucket) {
                    "ok"
                } else {
                    "skipped"
                },
            )
        })
        .collect();

    if let Some(existing) = existing_content
        && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(existing)
        && parsed.get("items") == Some(&serde_json::Value::Array(items_value.clone()))
        && parsed.get("toolchain").and_then(serde_json::Value::as_str) == Some(toolchain.as_str())
    {
        println!(
            "improve-scan: backlog unchanged ({} items); not rewriting",
            items.len()
        );
        return Ok(());
    }

    let payload = serde_json::json!({
        "version": 1,
        "generatedBy": "cargo run -p xtask -- improve-scan",
        "toolchain": toolchain,
        "generatedAt": unix_timestamp(),
        "buckets": buckets,
        "items": items_value,
    });
    let content = serde_json::to_string_pretty(&payload)
        .map(|content| content + "\n")
        .map_err(|error| error.to_string())?;
    if let Some(parent) = Path::new(BACKLOG_PATH).parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(BACKLOG_PATH, content).map_err(|error| error.to_string())?;
    println!("improve-scan: wrote {BACKLOG_PATH} ({} items)", items.len());
    Ok(())
}

fn rustc_version() -> String {
    std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map_or_else(
            || "unknown".to_string(),
            |output| String::from_utf8_lossy(&output.stdout).trim().to_string(),
        )
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, bucket: &str, status: &str, count: u64) -> Item {
        let mut item = Item::new(id, bucket, Vec::new(), id, String::new(), "S", count);
        item.status = status.to_string();
        item
    }

    fn all_buckets() -> BTreeSet<String> {
        BUCKET_ORDER
            .iter()
            .map(|bucket| (*bucket).to_string())
            .collect()
    }

    #[test]
    fn merge_preserves_blocked_and_in_review() {
        let existing = vec![
            item("lint:clippy::a", "lint-debt", "blocked", 5),
            item("lint:clippy::b", "lint-debt", "in-review", 3),
        ];
        let fresh = vec![
            item("lint:clippy::a", "lint-debt", "pending", 7),
            item("lint:clippy::b", "lint-debt", "pending", 3),
        ];
        let merged = merge_backlog(&existing, &BTreeMap::new(), fresh, &all_buckets());
        let by_id: BTreeMap<&str, &Item> =
            merged.iter().map(|item| (item.id.as_str(), item)).collect();
        assert_eq!(by_id["lint:clippy::a"].status, "blocked");
        assert_eq!(
            by_id["lint:clippy::a"].count, 7,
            "evidence refreshed in place"
        );
        assert_eq!(by_id["lint:clippy::b"].status, "in-review");
    }

    #[test]
    fn merge_reopens_done_on_reappearing_evidence() {
        let existing = vec![item("lint:clippy::a", "lint-debt", "done", 5)];
        let fresh = vec![item("lint:clippy::a", "lint-debt", "pending", 2)];
        let merged = merge_backlog(&existing, &BTreeMap::new(), fresh, &all_buckets());
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].status, "pending");
        assert_eq!(merged[0].reopened_from.as_deref(), Some("done"));
    }

    #[test]
    fn merge_resolves_vanished_then_prunes_next_cycle() {
        let existing = vec![item("lint:clippy::gone", "lint-debt", "pending", 5)];
        // Cycle 1: evidence vanished -> resolved (still listed).
        let cycle1 = merge_backlog(&existing, &BTreeMap::new(), Vec::new(), &all_buckets());
        assert_eq!(cycle1.len(), 1);
        assert_eq!(cycle1[0].status, "resolved");
        // Cycle 2: resolved item pruned.
        let cycle2 = merge_backlog(&cycle1, &BTreeMap::new(), Vec::new(), &all_buckets());
        assert!(cycle2.is_empty());
    }

    #[test]
    fn reappear_after_prune_returns_without_lineage() {
        let existing = vec![item("lint:clippy::flaky", "lint-debt", "pending", 5)];
        let cycle1 = merge_backlog(&existing, &BTreeMap::new(), Vec::new(), &all_buckets());
        let cycle2 = merge_backlog(&cycle1, &BTreeMap::new(), Vec::new(), &all_buckets());
        assert!(cycle2.is_empty());
        // Cycle 3: same id re-detected after prune -> fresh item, no marker.
        let fresh = vec![item("lint:clippy::flaky", "lint-debt", "pending", 4)];
        let cycle3 = merge_backlog(&cycle2, &BTreeMap::new(), fresh, &all_buckets());
        assert_eq!(cycle3.len(), 1);
        assert_eq!(cycle3[0].status, "pending");
        assert_eq!(
            cycle3[0].reopened_from, None,
            "history lives in the ledger only"
        );
    }

    #[test]
    fn merge_skipped_bucket_never_resolves_its_items() {
        let existing = vec![item("lint:clippy::a", "lint-debt", "pending", 5)];
        let mut scanned = BTreeSet::new();
        scanned.insert("provider-wiring".to_string()); // lint-debt NOT scanned
        let merged = merge_backlog(&existing, &BTreeMap::new(), Vec::new(), &scanned);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].status, "pending", "skipped bucket passes through");
    }

    #[test]
    fn ledger_outcomes_fold_before_merge() {
        let existing = vec![item("lint:clippy::a", "lint-debt", "in-review", 5)];
        let mut ledger = BTreeMap::new();
        ledger.insert("lint:clippy::a".to_string(), "done".to_string());
        // Evidence gone (the batch fixed it): done -> resolved this cycle.
        let merged = merge_backlog(&existing, &ledger, Vec::new(), &all_buckets());
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].status, "resolved");
        // Evidence still present (fix incomplete): done -> reopened pending.
        let fresh = vec![item("lint:clippy::a", "lint-debt", "pending", 1)];
        let merged = merge_backlog(&existing, &ledger, fresh, &all_buckets());
        assert_eq!(merged[0].status, "pending");
        assert_eq!(merged[0].reopened_from.as_deref(), Some("done"));
    }

    #[test]
    fn provider_normalization_matches_worked_example_oracle() {
        let dirs = vec![
            "binance".to_string(),
            "fileset".to_string(),
            "ws".to_string(),
            "ws-mock".to_string(),
            "yahoo".to_string(),
            "eia".to_string(),
        ];
        let mut keys = BTreeSet::new();
        keys.insert("provider-binance-http".to_string()); // wired via -http variant
        keys.insert("provider-eia".to_string()); // wired via plain key
        let mut has_http = BTreeMap::new();
        has_http.insert("yahoo".to_string(), true);
        has_http.insert("binance".to_string(), true);
        has_http.insert("fileset".to_string(), false);
        has_http.insert("ws".to_string(), false);

        let items = classify_providers(&dirs, &keys, &has_http);
        let by_id: BTreeMap<&str, &Item> =
            items.iter().map(|item| (item.id.as_str(), item)).collect();

        assert!(
            !by_id.contains_key("provider:binance"),
            "binance is wired (-http key)"
        );
        assert!(
            !by_id.contains_key("provider:eia"),
            "eia is wired (plain key)"
        );
        assert!(
            !by_id.contains_key("provider:ws-mock"),
            "ws-mock is excluded"
        );
        assert_eq!(
            by_id["provider:yahoo"].status, "pending",
            "yahoo is unwired-http"
        );
        assert!(by_id["provider:yahoo"].title.contains("Wire"));
        assert_eq!(by_id["provider:fileset"].status, "needs-design");
        assert_eq!(by_id["provider:ws"].status, "needs-design");
    }

    #[test]
    fn provider_feature_keys_ignore_list_entries() {
        let manifest = r#"
[features]
provider-eia = ["dep:tdw-provider-eia", "tdw-provider-eia/http"]
provider-alpha-vantage = [
    "dep:tdw-provider-alpha-vantage",
]
provider-binance-http = ["tdw-provider-binance/http"]
all-http-providers = [
    "provider-eia",
    "provider-binance-http",
]
"#;
        let keys = provider_feature_keys(manifest);
        assert!(keys.contains("provider-eia"));
        assert!(keys.contains("provider-alpha-vantage"));
        assert!(keys.contains("provider-binance-http"));
        assert!(!keys.contains("provider-yahoo"));
    }

    #[test]
    fn clippy_parse_tolerates_junk_lines() {
        let lines = [
            "not json at all {{{",
            r#"{"reason":"compiler-artifact","target":{"name":"tdw-core"}}"#,
            r#"{"reason":"compiler-message","message":{"level":"warning","code":{"code":"clippy::needless_pass_by_value"}},"target":{"name":"tdw-core"}}"#,
            r#"{"reason":"compiler-message","message":{"level":"warning","code":{"code":"clippy::needless_pass_by_value"}},"target":{"name":"tdw-core"}}"#,
            r#"{"reason":"compiler-message","message":{"level":"warning","code":null}}"#,
            r#"{"reason":"compiler-message"}"#,
            r#"{"reason":"compiler-message","message":{"level":"warning","code":{"code":"dead_code"}},"target":{"name":"tdw-bus"}}"#,
        ];
        let (counts, skipped) = parse_clippy_lines(lines.into_iter());
        assert_eq!(skipped, 2, "junk line + missing message field");
        assert_eq!(
            counts
                .get(&(
                    "clippy::needless_pass_by_value".to_string(),
                    "tdw-core".to_string()
                ))
                .copied(),
            Some(2)
        );
        assert!(
            !counts.keys().any(|(code, _)| code == "dead_code"),
            "non-clippy codes are out of scope"
        );
    }

    #[test]
    fn lint_items_group_per_lint_family() {
        let mut counts = BTreeMap::new();
        counts.insert(
            ("clippy::too_many_lines".to_string(), "tdw-core".to_string()),
            3,
        );
        counts.insert(
            ("clippy::too_many_lines".to_string(), "tdw-bus".to_string()),
            4,
        );
        counts.insert(
            (
                "clippy::option_if_let_else".to_string(),
                "tdw-kg".to_string(),
            ),
            60,
        );
        let items = lint_items(&counts);
        assert_eq!(items.len(), 2);
        let by_id: BTreeMap<&str, &Item> =
            items.iter().map(|item| (item.id.as_str(), item)).collect();
        let grouped = by_id["lint:clippy::too_many_lines"];
        assert_eq!(grouped.count, 7);
        assert_eq!(
            grouped.crates,
            vec!["tdw-bus".to_string(), "tdw-core".to_string()]
        );
        assert_eq!(grouped.effort, "S");
        assert_eq!(by_id["lint:clippy::option_if_let_else"].effort, "L");
    }

    #[test]
    fn ledger_front_matter_parses_and_tolerates_junk() {
        let content = r"---
batch: batch-lint-debt-001
items: lint:clippy::a, lint:clippy::b
outcome: done
---
# Batch report body (ignored)
";
        let outcomes = parse_ledger_front_matter(content);
        assert_eq!(
            outcomes,
            vec![
                ("lint:clippy::a".to_string(), "done".to_string()),
                ("lint:clippy::b".to_string(), "done".to_string()),
            ]
        );
        assert!(parse_ledger_front_matter("no front matter here").is_empty());
        assert!(
            parse_ledger_front_matter("---\nitems: x\noutcome: shipped\n---\n").is_empty(),
            "unknown outcomes are ignored"
        );
    }

    #[test]
    fn item_round_trips_through_json() {
        let mut original = Item::new(
            "provider:yahoo",
            "provider-wiring",
            vec!["tdw-provider-yahoo".to_string()],
            "Wire yahoo",
            "evidence".to_string(),
            "M",
            1,
        );
        original.reopened_from = Some("done".to_string());
        let value = original.to_value();
        let parsed = Item::from_value(&value).expect("round trip");
        assert_eq!(parsed, original);
    }

    #[test]
    fn merged_backlog_orders_by_bucket_then_count() {
        let fresh = vec![
            item("hygiene:todo-markers", "hygiene", "pending", 100),
            item("lint:clippy::a", "lint-debt", "pending", 2),
            item("lint:clippy::b", "lint-debt", "pending", 50),
            item("provider:yahoo", "provider-wiring", "pending", 1),
        ];
        let merged = merge_backlog(&[], &BTreeMap::new(), fresh, &all_buckets());
        let ids: Vec<&str> = merged.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "provider:yahoo",
                "lint:clippy::b",
                "lint:clippy::a",
                "hygiene:todo-markers",
            ]
        );
    }
}
