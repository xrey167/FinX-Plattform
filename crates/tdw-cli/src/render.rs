//! Result rendering and export (WS4 / gap-matrix items **L5.3** / **L5.4**).
//!
//! The daemon returns the terminal `EventMsg::Completed` whose `result` is the
//! `{ evidence, result: <ResultEnvelope> }` shape `dispatch_fetch_data` builds.
//! This module digs the standardized rows out of that envelope and renders them
//! three ways:
//! * a hand-rolled, aligned plain-text table (the default; no table-formatting
//!   dependency is pulled in),
//! * the raw envelope as pretty JSON (`--json`), and
//! * a CSV / JSON file export (`--export`).
//!
//! Export scope is deliberately CSV + JSON only: Parquet and XLSX are owned by
//! the `tdw-storage-parquet` / `tdw-table-format` export polish in gap-matrix
//! item **L5.4** and are out of scope for the CLI transport layer here.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

/// Pull the ordered list of record objects out of a terminal-event `result`.
///
/// Accepts the daemon's `{ evidence, result: { results: [...] } }` wrapper, a
/// bare `{ results: [...] }` envelope, or a top-level `[...]` array — whichever
/// the caller hands in — and returns each record as a JSON object map. Non-object
/// records are wrapped under a `"value"` key (mirroring
/// `ResultEnvelope::to_records`) so the row shape is uniform.
#[must_use]
pub fn records_from_result(result: &Value) -> Vec<Map<String, Value>> {
    let envelope = result.get("result").unwrap_or(result);
    let results = envelope
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| envelope.as_array().cloned())
        .unwrap_or_default();
    results
        .into_iter()
        .map(|item| match item {
            Value::Object(map) => map,
            other => {
                let mut map = Map::new();
                map.insert("value".to_string(), other);
                map
            }
        })
        .collect()
}

/// Column order across all records: keys in first-seen order, then any keys that
/// appear only in later records (sorted) so nothing is silently dropped.
#[must_use]
fn columns(records: &[Map<String, Value>]) -> Vec<String> {
    let mut order: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for record in records {
        for key in record.keys() {
            if seen.insert(key.clone()) {
                order.push(key.clone());
            }
        }
    }
    order
}

/// Render one JSON cell value as a compact display string for a table / CSV cell.
#[must_use]
fn cell(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => other.to_string(),
    }
}

/// Render `records` as an aligned plain-text table.
///
/// Column widths are the max of the header and every cell in that column. An
/// empty record set renders a single `(no rows)` line so the caller always emits
/// something deterministic.
#[must_use]
pub fn table(records: &[Map<String, Value>]) -> String {
    if records.is_empty() {
        return "(no rows)".to_string();
    }
    let cols = columns(records);
    let mut widths: Vec<usize> = cols.iter().map(String::len).collect();
    let rows: Vec<Vec<String>> = records
        .iter()
        .map(|record| {
            cols.iter()
                .enumerate()
                .map(|(i, key)| {
                    let text = cell(record.get(key));
                    if text.len() > widths[i] {
                        widths[i] = text.len();
                    }
                    text
                })
                .collect()
        })
        .collect();

    let mut out = String::new();
    push_row(&mut out, &cols, &widths);
    let separators: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
    push_row(&mut out, &separators, &widths);
    for row in &rows {
        push_row(&mut out, row, &widths);
    }
    out
}

/// Append one space-padded, ` | `-joined row line to `out`.
///
/// Every cell (including the last) is padded to its column width so all lines —
/// header, separator, and data rows — share one visible width and stay aligned.
fn push_row(out: &mut String, cells: &[String], widths: &[usize]) {
    let line: Vec<String> = cells
        .iter()
        .enumerate()
        .map(|(i, text)| {
            format!(
                "{text:<width$}",
                width = widths.get(i).copied().unwrap_or(0)
            )
        })
        .collect();
    out.push_str(&line.join(" | "));
    out.push('\n');
}

/// Escape one CSV field per RFC 4180: wrap in double quotes and double any inner
/// quote when the field contains a comma, quote, CR, or LF.
#[must_use]
pub fn csv_escape(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// Render `records` as an RFC-4180 CSV document (header row + one row per record).
#[must_use]
pub fn to_csv(records: &[Map<String, Value>]) -> String {
    let cols = columns(records);
    let mut out = String::new();
    out.push_str(
        &cols
            .iter()
            .map(|c| csv_escape(c))
            .collect::<Vec<_>>()
            .join(","),
    );
    out.push('\n');
    for record in records {
        let line: Vec<String> = cols
            .iter()
            .map(|key| csv_escape(&cell(record.get(key))))
            .collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    out
}

/// Render `records` as a pretty-printed JSON array.
///
/// # Errors
///
/// Returns [`serde_json::Error`] if the records fail to serialize (they are
/// already `serde_json` values, so this is effectively infallible).
pub fn to_json(records: &[Map<String, Value>]) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_records() -> Vec<Map<String, Value>> {
        records_from_result(&json!({
            "evidence": {"ok": true},
            "result": {
                "id": "req-1",
                "results": [
                    {"symbol": "AAPL", "close": 191.5, "open": 190.0},
                    {"symbol": "AAPL", "close": 193.25, "open": 191.5}
                ],
                "provider": "yahoo"
            }
        }))
    }

    #[test]
    fn records_extracted_from_wrapped_envelope() {
        let records = sample_records();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].get("symbol"), Some(&json!("AAPL")));
        assert_eq!(records[1].get("close"), Some(&json!(193.25)));
    }

    #[test]
    fn records_extracted_from_bare_envelope_and_array() {
        let bare = records_from_result(&json!({"results": [{"a": 1}]}));
        assert_eq!(bare.len(), 1);
        let array = records_from_result(&json!([{"a": 1}, {"a": 2}]));
        assert_eq!(array.len(), 2);
    }

    #[test]
    fn scalar_records_wrap_under_value_key() {
        let records = records_from_result(&json!({"results": [1, 2, 3]}));
        assert_eq!(records.len(), 3);
        assert_eq!(records[0].get("value"), Some(&json!(1)));
    }

    #[test]
    fn table_aligns_columns_and_has_header_and_separator() {
        let rendered = table(&sample_records());
        let lines: Vec<&str> = rendered.lines().collect();
        // header, separator, 2 data rows.
        assert_eq!(lines.len(), 4);
        // Columns are key-sorted (serde_json maps order keys): close, open, symbol.
        assert!(lines[0].contains("close"));
        assert!(lines[0].contains("symbol"));
        assert!(lines[1].starts_with("-----"));
        assert!(lines[2].contains("AAPL"));
        // every line is the same visible width (aligned).
        let width = lines[0].len();
        for line in &lines {
            assert_eq!(line.len(), width, "ragged line: {line:?}");
        }
    }

    #[test]
    fn empty_records_render_no_rows_marker() {
        assert_eq!(table(&[]), "(no rows)");
    }

    #[test]
    fn csv_escapes_commas_quotes_and_newlines() {
        assert_eq!(csv_escape("plain"), "plain");
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        assert_eq!(csv_escape("a\"b"), "\"a\"\"b\"");
        assert_eq!(csv_escape("a\nb"), "\"a\nb\"");
    }

    #[test]
    fn csv_round_trips_header_and_rows() {
        let csv = to_csv(&sample_records());
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 3, "header + 2 rows");
        // Key-sorted columns: close,open,symbol.
        assert_eq!(lines[0], "close,open,symbol");
        assert!(lines[1].contains("AAPL"));
    }

    #[test]
    fn json_export_is_an_array_of_objects() {
        let out = to_json(&sample_records()).expect("serialize");
        let parsed: Value = serde_json::from_str(&out).expect("parse");
        assert!(parsed.is_array());
        assert_eq!(parsed.as_array().map(Vec::len), Some(2));
    }
}
