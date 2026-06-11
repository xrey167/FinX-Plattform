//! Event-spine routines: record / run / list (WS4 / gap-matrix item **L5.3**).
//!
//! A routine is a replayable script of `OpEnvelope`s, stored as one JSON object
//! per line (JSONL) under `.tdw/routines/<name>.jsonl` in the working directory.
//! This is strictly local-file state — there is no daemon-side routine registry —
//! which keeps replay portable and inspectable (the file is plain JSON envelopes
//! you can read, diff, and edit by hand).
//!
//! * `routine record <name> -- <subcommand...>` runs the wrapped command and
//!   appends its `OpEnvelope` to the routine file. (The append is performed by
//!   the caller after it builds the envelope; this module owns the file format.)
//! * `routine run <name> [--var k=v ...]` reads each stored envelope, applies
//!   `${k}` -> `v` substitution across its `params`, mints a fresh op/session id,
//!   and re-submits it.
//! * `routine list` enumerates the recorded routine names.
//!
//! Substitution is a literal `${key}` -> `value` textual replace over the
//! serialized params object, so it works uniformly for string, number, and
//! nested values without a templating dependency.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Directory (relative to CWD) that holds per-routine JSONL files.
pub const ROUTINES_DIR: &str = ".tdw/routines";

/// Absolute-or-relative path to the JSONL file for routine `name`.
#[must_use]
pub fn routine_path(name: &str) -> PathBuf {
    Path::new(ROUTINES_DIR).join(format!("{name}.jsonl"))
}

/// Append an envelope line to routine `name` under the CWD's routines dir.
///
/// # Errors
///
/// Returns an [`std::io::Error`] if the directory or file cannot be created or
/// written.
pub fn append_envelope_line(name: &str, envelope_json: &str) -> std::io::Result<()> {
    append_envelope_line_in(Path::new(ROUTINES_DIR), name, envelope_json)
}

/// Read the recorded envelope lines for routine `name` from the CWD's dir.
///
/// # Errors
///
/// Returns an error string if the file cannot be read or any line is not valid
/// JSON.
pub fn read_envelopes(name: &str) -> Result<Vec<Value>, String> {
    read_envelopes_in(Path::new(ROUTINES_DIR), name)
}

/// List recorded routine names under the CWD's routines dir.
///
/// # Errors
///
/// Returns an error string if the directory exists but cannot be read.
pub fn list_routines() -> Result<Vec<String>, String> {
    list_routines_in(Path::new(ROUTINES_DIR))
}

/// Append one already-serialized envelope JSON line to routine `name` under
/// `dir`, creating the directory and file as needed.
///
/// The CWD-relative [`append_envelope_line`] delegates here; tests call this
/// directly with a temp dir to stay hermetic without mutating the process CWD.
///
/// # Errors
///
/// Returns an [`std::io::Error`] if the directory or file cannot be created or
/// written.
pub fn append_envelope_line_in(dir: &Path, name: &str, envelope_json: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{name}.jsonl"));
    let mut existing = std::fs::read_to_string(&path).unwrap_or_default();
    existing.push_str(envelope_json.trim_end());
    existing.push('\n');
    std::fs::write(&path, existing)
}

/// Read the recorded envelope lines for routine `name` from `dir`.
///
/// Blank lines are skipped. A line that fails to parse is surfaced as an error so
/// a corrupt routine file fails loudly rather than silently dropping a step.
///
/// # Errors
///
/// Returns an error string if the file cannot be read or any line is not valid
/// JSON.
pub fn read_envelopes_in(dir: &Path, name: &str) -> Result<Vec<Value>, String> {
    let path = dir.join(format!("{name}.jsonl"));
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("read routine {}: {e}", path.display()))?;
    let mut out = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(trimmed)
            .map_err(|e| format!("routine {name} line {}: {e}", idx + 1))?;
        out.push(value);
    }
    Ok(out)
}

/// List recorded routine names (`*.jsonl` file stems) under `dir`, sorted.
///
/// Returns an empty list when the directory does not exist yet.
///
/// # Errors
///
/// Returns an error string if the directory exists but cannot be read.
pub fn list_routines_in(dir: &Path) -> Result<Vec<String>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read dir {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("read entry: {e}"))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("jsonl")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            names.push(stem.to_string());
        }
    }
    names.sort();
    Ok(names)
}

/// Parse `--var key=value` pairs into a substitution map.
///
/// Each entry must contain a single `=`; the portion before it is the key, the
/// remainder (which may itself contain `=`) is the value.
///
/// # Errors
///
/// Returns an error string for an entry with no `=` separator.
pub fn parse_vars(pairs: &[String]) -> Result<BTreeMap<String, String>, String> {
    let mut map = BTreeMap::new();
    for pair in pairs {
        let Some((key, value)) = pair.split_once('=') else {
            return Err(format!("--var must be key=value, got {pair:?}"));
        };
        map.insert(key.to_string(), value.to_string());
    }
    Ok(map)
}

/// Apply `${key}` -> `value` substitution across a params value.
///
/// The params object is serialized, every `${key}` token is literally replaced
/// with its mapped value, and the result is reparsed. When no token is present
/// the input is returned unchanged. Reparsing keeps the value typed (a numeric
/// substitution stays a JSON string only if it was quoted in the source).
#[must_use]
pub fn substitute(params: &Value, vars: &BTreeMap<String, String>) -> Value {
    if vars.is_empty() {
        return params.clone();
    }
    let Ok(mut text) = serde_json::to_string(params) else {
        return params.clone();
    };
    for (key, value) in vars {
        text = text.replace(&format!("${{{key}}}"), value);
    }
    serde_json::from_str(&text).unwrap_or_else(|_| params.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A unique temp directory for one hermetic routine test. No process CWD is
    /// mutated, so tests run safely in parallel (the harness default).
    fn temp_dir(tag: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        dir.push(format!("tdw-routine-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).expect("mk temp dir");
        dir
    }

    #[test]
    fn record_then_read_round_trips_envelopes() {
        let dir = temp_dir("rt");
        let env_a = json!({"op": {"fetch_data": {"route": "equity/price/historical"}}});
        let env_b = json!({"op": {"fetch_data": {"route": "crypto/price/historical"}}});
        append_envelope_line_in(&dir, "daily", &env_a.to_string()).expect("append a");
        append_envelope_line_in(&dir, "daily", &env_b.to_string()).expect("append b");

        let envelopes = read_envelopes_in(&dir, "daily").expect("read");
        assert_eq!(envelopes.len(), 2);
        assert_eq!(envelopes[0], env_a);
        assert_eq!(envelopes[1], env_b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_routines_reports_recorded_names_sorted() {
        let dir = temp_dir("list");
        append_envelope_line_in(&dir, "zeta", "{}").expect("append zeta");
        append_envelope_line_in(&dir, "alpha", "{}").expect("append alpha");
        let names = list_routines_in(&dir).expect("list");
        assert_eq!(names, vec!["alpha".to_string(), "zeta".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_routines_empty_when_absent() {
        let dir = std::env::temp_dir().join("tdw-routine-absent-does-not-exist-xyz");
        assert!(list_routines_in(&dir).expect("list").is_empty());
    }

    #[test]
    fn parse_vars_splits_on_first_equals() {
        let vars = parse_vars(&["symbol=AAPL".to_string(), "q=a=b".to_string()]).expect("parse");
        assert_eq!(vars.get("symbol"), Some(&"AAPL".to_string()));
        assert_eq!(vars.get("q"), Some(&"a=b".to_string()));
        assert!(parse_vars(&["bad".to_string()]).is_err());
    }

    #[test]
    fn substitute_replaces_tokens_in_params() {
        let params = json!({"symbol": "${sym}", "provider": "yahoo"});
        let vars = parse_vars(&["sym=MSFT".to_string()]).expect("parse");
        let out = substitute(&params, &vars);
        assert_eq!(out.get("symbol"), Some(&json!("MSFT")));
        assert_eq!(out.get("provider"), Some(&json!("yahoo")));
    }

    #[test]
    fn substitute_no_vars_is_identity() {
        let params = json!({"symbol": "AAPL"});
        let out = substitute(&params, &BTreeMap::new());
        assert_eq!(out, params);
    }
}
