//! Pure Op-construction layer (WS4 / gap-matrix item **L5.3**).
//!
//! Translates a resolved clap leaf `ArgMatches` for a catalog route into the
//! `params` JSON object the daemon's `Op::FetchData` consumes. Kept dependency-
//! and IO-free so it can be unit-tested without a daemon: the live end-to-end
//! path (an actual TCP submit) is covered by the `--smoke` harness, not here.
//!
//! Mapping rules (mirrors `tree::build_leaf`):
//! * `--symbol`, `--provider`, and every schema-derived `--<name>` value flag are
//!   inserted into the params object under their own key, typed by the route's
//!   [`ParamKind`] (numbers parse to JSON numbers; everything else stays a
//!   string),
//! * boolean flags are inserted as `true` only when present, and
//! * `--chart` is forwarded as `"chart": true` (the chart envelope slot is G014,
//!   in-flight; an older dispatcher simply ignores the key).

use clap::ArgMatches;
use serde_json::{Map, Value};
use tdw_endpoint_catalog::CatalogEntry;

use crate::tree::{ParamKind, params_for};

/// Build the `params` object for a route from its resolved leaf matches.
///
/// Only flags the user actually supplied are inserted, so the params object
/// carries exactly the caller's intent (the daemon fills defaults). `--symbol`
/// and `--provider` are handled here explicitly because they are not part of the
/// shared params schema.
#[must_use]
pub fn build_params(entry: &CatalogEntry, matches: &ArgMatches) -> Value {
    let mut params: Map<String, Value> = Map::new();

    if let Some(symbol) = matches.get_one::<String>("symbol") {
        params.insert("symbol".to_string(), Value::String(symbol.clone()));
    }
    // `--provider` exists only on Fetch leaves; `try_get_one` avoids clap's panic
    // on Compute leaves where the arg was never defined.
    if let Ok(Some(provider)) = matches.try_get_one::<String>("provider") {
        params.insert("provider".to_string(), Value::String(provider.clone()));
    }

    for param in params_for(entry) {
        match param.kind {
            ParamKind::Bool => {
                if matches.get_flag(param.name.as_str()) {
                    params.insert(param.name.clone(), Value::Bool(true));
                }
            }
            ParamKind::Num | ParamKind::Str => {
                if let Some(raw) = matches.get_one::<String>(param.name.as_str()) {
                    params.insert(param.name.clone(), coerce(raw, param.kind));
                }
            }
        }
    }

    if matches.get_flag("chart") {
        params.insert("chart".to_string(), Value::Bool(true));
    }

    Value::Object(params)
}

/// Coerce a raw string flag value to a JSON value by its declared kind.
///
/// A numeric kind that fails to parse falls back to a JSON string so the daemon
/// can surface a precise validation error rather than the CLI guessing.
fn coerce(raw: &str, kind: ParamKind) -> Value {
    match kind {
        ParamKind::Num => raw.parse::<i64>().map_or_else(
            |_| {
                raw.parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
                    .map_or_else(|| Value::String(raw.to_string()), Value::Number)
            },
            Value::from,
        ),
        ParamKind::Str | ParamKind::Bool => Value::String(raw.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::build_root;
    use serde_json::json;

    /// Resolve `argv` against the full built CLI and return the leaf matches plus
    /// the matched route's catalog entry.
    fn resolve(argv: &[&str]) -> (CatalogEntry, ArgMatches) {
        let matches = build_root()
            .try_get_matches_from(argv)
            .expect("argv should parse");
        // Walk the subcommand chain to the leaf, accumulating the route.
        let mut cursor = &matches;
        let mut segments: Vec<String> = Vec::new();
        while let Some((name, sub)) = cursor.subcommand() {
            segments.push(name.to_string());
            cursor = sub;
        }
        let route = segments.join("/");
        let entry = tdw_endpoint_catalog::lookup(&route)
            .unwrap_or_else(|| panic!("resolved route {route} not in catalog"));
        (entry, cursor.clone())
    }

    #[test]
    fn fetch_route_builds_symbol_provider_and_schema_params() {
        let (entry, matches) = resolve(&[
            "tdw",
            "equity",
            "price",
            "historical",
            "--symbol",
            "AAPL",
            "--provider",
            "yahoo",
            "--limit",
            "30",
            "--interval",
            "1d",
        ]);
        let params = build_params(&entry, &matches);
        assert_eq!(params.get("symbol"), Some(&json!("AAPL")));
        assert_eq!(params.get("provider"), Some(&json!("yahoo")));
        // `limit` is numeric -> JSON number.
        assert_eq!(params.get("limit"), Some(&json!(30)));
        // `interval` is an enum token -> string.
        assert_eq!(params.get("interval"), Some(&json!("1d")));
    }

    #[test]
    fn omitted_flags_are_absent_from_params() {
        let (entry, matches) =
            resolve(&["tdw", "equity", "price", "historical", "--symbol", "AAPL"]);
        let params = build_params(&entry, &matches);
        let obj = params.as_object().expect("object");
        assert!(obj.contains_key("symbol"));
        assert!(!obj.contains_key("limit"));
        assert!(!obj.contains_key("provider"));
        assert!(!obj.contains_key("chart"));
    }

    #[test]
    fn chart_flag_forwards_as_params_chart_true() {
        let (entry, matches) = resolve(&[
            "tdw",
            "equity",
            "price",
            "historical",
            "--symbol",
            "AAPL",
            "--chart",
        ]);
        let params = build_params(&entry, &matches);
        assert_eq!(params.get("chart"), Some(&json!(true)));
    }

    #[test]
    fn compute_route_builds_params_without_provider() {
        let (entry, matches) = resolve(&["tdw", "technical", "sma", "--symbol", "AAPL"]);
        let params = build_params(&entry, &matches);
        assert_eq!(params.get("symbol"), Some(&json!("AAPL")));
        assert!(
            params
                .as_object()
                .is_some_and(|o| !o.contains_key("provider"))
        );
    }
}
