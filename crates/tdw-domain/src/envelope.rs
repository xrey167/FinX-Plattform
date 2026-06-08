//! Standardized result envelope (gap-matrix item **L1.1**).
//!
//! Every `tdw_core::Fetcher` result is wrapped in a [`ResultEnvelope<T>`], giving
//! callers (CLI, REST/SSE service, MCP, LLM/dataframe consumers) one consistent
//! shape regardless of which provider served the request. The field shape mirrors
//! the documented OBBject surface (`id` / `results` / `provider` / `warnings` /
//! `extra{route, timestamp, arguments}`) as enumerated in
//! `docs/roadmap/openbb-surface-domains.md` — names are FinX's own, derived from the
//! public surface docs, not from any upstream source code.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A non-fatal warning attached to a result (e.g. a provider degraded a field,
/// clamped a date range, or fell back to a cached value).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Warning {
    /// Machine-readable category, e.g. `"provider"` or `"deprecation"`.
    pub category: String,
    /// Human-readable message describing the warning.
    pub message: String,
}

impl Warning {
    /// Construct a warning from a category and message.
    pub fn new(category: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            message: message.into(),
        }
    }
}

/// Side-channel metadata describing *how* a result was produced.
///
/// Mirrors the documented `extra` object: the logical `route` that was called,
/// the `timestamp` the result was assembled, and the normalized `arguments` the
/// request resolved to. `arguments` is a stable-ordered string map so snapshots
/// are deterministic.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResultExtra {
    /// Logical command route, e.g. `"equity/price/historical"`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub route: Option<String>,
    /// RFC 3339 timestamp the envelope was assembled (stored as a string to match
    /// the timestamp convention used elsewhere in this crate).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub timestamp: Option<String>,
    /// Normalized request arguments, stable-ordered for deterministic snapshots.
    #[serde(default)]
    pub arguments: BTreeMap<String, String>,
}

impl ResultExtra {
    /// Set the logical route (builder style).
    #[must_use]
    pub fn with_route(mut self, route: impl Into<String>) -> Self {
        self.route = Some(route.into());
        self
    }

    /// Set the assembly timestamp (builder style).
    #[must_use]
    pub fn with_timestamp(mut self, timestamp: impl Into<String>) -> Self {
        self.timestamp = Some(timestamp.into());
        self
    }

    /// Insert a normalized argument (builder style).
    #[must_use]
    pub fn with_argument(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.arguments.insert(key.into(), value.into());
        self
    }
}

/// Standardized result envelope wrapping any provider-normalized record type `T`.
///
/// This is the single shape returned across every logical endpoint, so clients can
/// treat results uniformly: `results` carries the standardized rows, `provider`
/// records which source served them, `warnings` surface non-fatal issues, and
/// `extra` carries route/timestamp/arguments provenance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResultEnvelope<T> {
    /// Stable identifier for this result (request id / correlation id).
    pub id: String,
    /// The standardized records produced for the request.
    pub results: Vec<T>,
    /// The provider key that served the request (e.g. `"yahoo"`, `"fmp"`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub provider: Option<String>,
    /// Non-fatal warnings accumulated while producing the result.
    #[serde(default)]
    pub warnings: Vec<Warning>,
    /// Route / timestamp / arguments provenance.
    #[serde(default)]
    pub extra: ResultExtra,
}

impl<T> ResultEnvelope<T> {
    /// Create an envelope with an id and the standardized results.
    pub fn new(id: impl Into<String>, results: Vec<T>) -> Self {
        Self {
            id: id.into(),
            results,
            provider: None,
            warnings: Vec::new(),
            extra: ResultExtra::default(),
        }
    }

    /// Set the serving provider (builder style).
    #[must_use]
    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    /// Replace the `extra` provenance block (builder style).
    #[must_use]
    pub fn with_extra(mut self, extra: ResultExtra) -> Self {
        self.extra = extra;
        self
    }

    /// Append a warning (builder style).
    #[must_use]
    pub fn with_warning(mut self, warning: Warning) -> Self {
        self.warnings.push(warning);
        self
    }

    /// Number of standardized records carried by the envelope.
    #[must_use]
    pub fn len(&self) -> usize {
        self.results.len()
    }

    /// Whether the envelope carries no records.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.results.is_empty()
    }
}

impl<T: Serialize> ResultEnvelope<T> {
    /// Flatten the envelope's `results` into a vector of JSON objects ("records"),
    /// the shape consumed by dataframe / table / export layers.
    ///
    /// Each record is the `serde_json` object form of one `T`. Non-object records
    /// (e.g. a `T` that serializes to a scalar) are wrapped under a `"value"` key so
    /// the output is always a uniform list of objects.
    ///
    /// # Errors
    ///
    /// Returns [`serde_json::Error`] if any record fails to serialize.
    pub fn to_records(
        &self,
    ) -> Result<Vec<serde_json::Map<String, serde_json::Value>>, serde_json::Error> {
        let mut records = Vec::with_capacity(self.results.len());
        for item in &self.results {
            let value = serde_json::to_value(item)?;
            match value {
                serde_json::Value::Object(map) => records.push(map),
                other => {
                    let mut map = serde_json::Map::new();
                    map.insert("value".to_string(), other);
                    records.push(map);
                }
            }
        }
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    struct Row {
        symbol: String,
        rank: u32,
    }

    fn sample() -> ResultEnvelope<Row> {
        ResultEnvelope::new(
            "req-1",
            vec![
                Row {
                    symbol: "AAPL".to_string(),
                    rank: 1,
                },
                Row {
                    symbol: "MSFT".to_string(),
                    rank: 2,
                },
            ],
        )
        .with_provider("yahoo")
        .with_warning(Warning::new("provider", "extended hours unavailable"))
        .with_extra(
            ResultExtra::default()
                .with_route("equity/price/historical")
                .with_timestamp("2026-06-07T00:00:00Z")
                .with_argument("symbol", "AAPL")
                .with_argument("interval", "1d"),
        )
    }

    #[test]
    fn builders_assemble_expected_shape() {
        let env = sample();
        assert_eq!(env.id, "req-1");
        assert_eq!(env.len(), 2);
        assert!(!env.is_empty());
        assert_eq!(env.provider.as_deref(), Some("yahoo"));
        assert_eq!(env.warnings.len(), 1);
        assert_eq!(env.extra.route.as_deref(), Some("equity/price/historical"));
        // BTreeMap keeps arguments stable-ordered.
        let keys: Vec<&String> = env.extra.arguments.keys().collect();
        assert_eq!(keys, vec!["interval", "symbol"]);
    }

    #[test]
    fn empty_envelope_reports_empty() {
        let env: ResultEnvelope<Row> = ResultEnvelope::new("req-empty", Vec::new());
        assert!(env.is_empty());
        assert_eq!(env.len(), 0);
    }

    #[test]
    fn serde_round_trip_preserves_all_fields() {
        let env = sample();
        let json = serde_json::to_string(&env).expect("serialize envelope");
        let back: ResultEnvelope<Row> = serde_json::from_str(&json).expect("deserialize envelope");
        assert_eq!(env, back);
    }

    #[test]
    fn to_records_flattens_objects() {
        let env = sample();
        let records = env.to_records().expect("flatten to records");
        assert_eq!(records.len(), 2);
        assert_eq!(
            records[0].get("symbol"),
            Some(&serde_json::Value::String("AAPL".to_string()))
        );
        assert_eq!(
            records[1].get("rank"),
            Some(&serde_json::Value::Number(2u32.into()))
        );
    }

    #[test]
    fn to_records_wraps_scalar_results() {
        let env: ResultEnvelope<u32> = ResultEnvelope::new("req-scalar", vec![1, 2, 3]);
        let records = env.to_records().expect("flatten scalars");
        assert_eq!(records.len(), 3);
        assert_eq!(
            records[0].get("value"),
            Some(&serde_json::Value::Number(1u32.into()))
        );
    }

    #[test]
    fn snapshot_json_is_stable() {
        // Snapshot-style assertion: the exact serialized form is load-bearing for
        // cross-client consistency, so pin it.
        let env = sample();
        let json = serde_json::to_string_pretty(&env).expect("serialize envelope");
        let expected = r#"{
  "id": "req-1",
  "results": [
    {
      "symbol": "AAPL",
      "rank": 1
    },
    {
      "symbol": "MSFT",
      "rank": 2
    }
  ],
  "provider": "yahoo",
  "warnings": [
    {
      "category": "provider",
      "message": "extended hours unavailable"
    }
  ],
  "extra": {
    "route": "equity/price/historical",
    "timestamp": "2026-06-07T00:00:00Z",
    "arguments": {
      "interval": "1d",
      "symbol": "AAPL"
    }
  }
}"#;
        assert_eq!(json, expected);
    }

    #[test]
    fn omitted_optionals_are_absent_in_json() {
        let env: ResultEnvelope<Row> = ResultEnvelope::new("req-min", Vec::new());
        let json = serde_json::to_string(&env).expect("serialize minimal envelope");
        // No provider, no route/timestamp set => those keys are skipped.
        assert!(!json.contains("\"provider\""));
        assert!(!json.contains("\"route\""));
        assert!(!json.contains("\"timestamp\""));
        // results / warnings / extra.arguments still present (default-emitting).
        assert!(json.contains("\"results\":[]"));
        assert!(json.contains("\"warnings\":[]"));
    }
}
