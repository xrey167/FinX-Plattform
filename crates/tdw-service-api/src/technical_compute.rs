//! Compute-route execution for the `technical/*` catalog namespace (gap-matrix
//! item **L4.1** wiring; the daemon side of the analytics crate).
//!
//! A `technical/*` route is an [`EndpointKind::Compute`] derivation: it has no
//! provider candidates. Instead of fetching, it runs a pure indicator from
//! `tdw-analytics-technical` over an OHLCV series the caller supplies. The bar
//! series itself is resolved by the dispatcher — either from an inline `data`
//! array or by first running a nested `source` Fetch route — and handed here as
//! parsed [`MarketDataBar`]s; this module is pure (bars + params → JSON rows),
//! so it carries no policy, I/O, or async.
//!
//! Each indicator is registered once. The registry is the single source of
//! truth for *which* `technical/*` routes have a compute implementation; a
//! conformance test in this crate proves it is exactly the set of `Compute`
//! routes the catalog declares, and the [`crate::dispatcher`] tool-registry
//! wiring registers one `RegisteredTool` per registry entry so `Op::ToolCall`
//! and MCP expose them for free.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;
use tdw_core::Error;
use tdw_domain::{MarketDataBar, TimeGranularity};

use tdw_analytics_technical::indicators::{bands, moving_average, oscillators, trend, volume};
use tdw_analytics_technical::params::{
    AdxParams, AroonParams, BollingerParams, CciParams, DonchianParams, FisherParams, HmaParams,
    KeltnerParams, LengthParams, MacdParams, RocParams, StochasticParams,
};

/// A compute implementation: parse the route's typed params from `params`, run
/// the indicator over `bars`, and serialize each output row to JSON. The
/// returned vec is positionally date-aligned with `bars` (one row per bar).
type ComputeFn = fn(&[MarketDataBar], &Value) -> Result<Vec<Value>, Error>;

/// Build the `route -> ComputeFn` registry for every `technical/*` indicator.
///
/// `BTreeMap` keeps iteration deterministic (used by the conformance test and
/// the tool-registry wiring). Routes match the catalog's `technical/*` entries.
#[must_use]
pub fn compute_registry() -> BTreeMap<&'static str, ComputeFn> {
    let mut registry: BTreeMap<&'static str, ComputeFn> = BTreeMap::new();
    registry.insert("technical/sma", compute_sma);
    registry.insert("technical/ema", compute_ema);
    registry.insert("technical/wma", compute_wma);
    registry.insert("technical/hma", compute_hma);
    registry.insert("technical/macd", compute_macd);
    registry.insert("technical/rsi", compute_rsi);
    registry.insert("technical/stochastic", compute_stochastic);
    registry.insert("technical/cci", compute_cci);
    registry.insert("technical/adx", compute_adx);
    registry.insert("technical/aroon", compute_aroon);
    registry.insert("technical/bbands", compute_bbands);
    registry.insert("technical/keltner", compute_keltner);
    registry.insert("technical/donchian", compute_donchian);
    registry.insert("technical/atr", compute_atr);
    registry.insert("technical/obv", compute_obv);
    registry.insert("technical/ad", compute_ad);
    registry.insert("technical/vwap", compute_vwap);
    registry.insert("technical/fisher", compute_fisher);
    registry.insert("technical/roc", compute_roc);
    registry.insert("technical/momentum", compute_momentum);
    registry
}

/// Run the compute implementation for `route` over `bars`.
///
/// # Errors
///
/// Returns [`Error::Provider`] when `route` has no registered compute
/// implementation, and [`Error::InvalidQuery`] when the supplied params are
/// invalid for the indicator (e.g. a zero period).
pub fn run_compute(
    route: &str,
    bars: &[MarketDataBar],
    params: &Value,
) -> Result<Vec<Value>, Error> {
    let registry = compute_registry();
    let Some(compute) = registry.get(route) else {
        return Err(Error::Provider(format!(
            "no compute implementation registered for catalog route {route}"
        )));
    };
    compute(bars, params)
}

/// A flexible inbound OHLCV record. Accepts either the `MarketDataBar` shape
/// (`ts` + `venue` + `granularity` + …) or the `EquityHistoricalData` /
/// generic-fetch shape (`date` + integer `volume`), so a nested fetch result
/// and a hand-rolled inline `data` array both parse without the caller
/// reshaping. Only the OHLCV fields are load-bearing for the math.
#[derive(Debug, Deserialize)]
struct InboundBar {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    venue: Option<String>,
    #[serde(default)]
    ts: Option<String>,
    #[serde(default)]
    date: Option<String>,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    #[serde(default)]
    volume: f64,
    #[serde(default)]
    source: Option<String>,
}

impl InboundBar {
    fn into_bar(self) -> MarketDataBar {
        MarketDataBar {
            symbol: self.symbol.unwrap_or_default(),
            venue: self.venue.unwrap_or_default(),
            granularity: TimeGranularity::Day,
            ts: self.ts.or(self.date).unwrap_or_default(),
            open: self.open,
            high: self.high,
            low: self.low,
            close: self.close,
            volume: self.volume,
            source: self.source.unwrap_or_default(),
        }
    }
}

/// Parse a JSON array of OHLCV records into [`MarketDataBar`]s.
///
/// # Errors
///
/// Returns [`Error::InvalidQuery`] when `records` is not an array or any element
/// lacks the required OHLC float fields.
pub fn parse_bars(records: &Value) -> Result<Vec<MarketDataBar>, Error> {
    let array = records.as_array().ok_or_else(|| {
        Error::InvalidQuery(
            "technical compute: `data` must be an array of OHLCV records".to_string(),
        )
    })?;
    let mut bars = Vec::with_capacity(array.len());
    for (index, record) in array.iter().enumerate() {
        let inbound: InboundBar = serde_json::from_value(record.clone()).map_err(|error| {
            Error::InvalidQuery(format!(
                "technical compute: row {index} is not a valid OHLCV record: {error}"
            ))
        })?;
        bars.push(inbound.into_bar());
    }
    Ok(bars)
}

// --- Param parsing -----------------------------------------------------------

/// Deserialize an indicator param struct from the request `params`, tolerating
/// the extra routing keys (`provider`, `data`, `source`, `symbol`) the dispatch
/// envelope carries by deserializing field-by-field via `serde(default)`.
fn parse_params<P: for<'de> Deserialize<'de> + Default>(params: &Value) -> Result<P, Error> {
    if params.is_object() {
        serde_json::from_value(params.clone()).map_err(|error| {
            Error::InvalidQuery(format!("technical compute: invalid params: {error}"))
        })
    } else {
        Ok(P::default())
    }
}

fn map_period_err(error: &tdw_analytics_technical::IndicatorError) -> Error {
    Error::InvalidQuery(format!("technical compute: {error}"))
}

fn closes(bars: &[MarketDataBar]) -> Vec<f64> {
    bars.iter().map(|b| b.close).collect()
}

fn rows_scalar(values: &[Option<f64>]) -> Result<Vec<Value>, Error> {
    values
        .iter()
        .map(|v| {
            serde_json::to_value(v).map_err(|e| Error::Provider(format!("compute serialize: {e}")))
        })
        .collect()
}

fn rows_struct<T: serde::Serialize>(values: &[T]) -> Result<Vec<Value>, Error> {
    values
        .iter()
        .map(|v| {
            serde_json::to_value(v).map_err(|e| Error::Provider(format!("compute serialize: {e}")))
        })
        .collect()
}

// --- Indicator adapters ------------------------------------------------------

fn compute_sma(bars: &[MarketDataBar], params: &Value) -> Result<Vec<Value>, Error> {
    let p: LengthParams = parse_params(params)?;
    rows_scalar(&moving_average::sma(&closes(bars), p).map_err(|e| map_period_err(&e))?)
}

fn compute_ema(bars: &[MarketDataBar], params: &Value) -> Result<Vec<Value>, Error> {
    let p: LengthParams = parse_params(params)?;
    rows_scalar(&moving_average::ema(&closes(bars), p).map_err(|e| map_period_err(&e))?)
}

fn compute_wma(bars: &[MarketDataBar], params: &Value) -> Result<Vec<Value>, Error> {
    let p: LengthParams = parse_params(params)?;
    rows_scalar(&moving_average::wma(&closes(bars), p).map_err(|e| map_period_err(&e))?)
}

fn compute_hma(bars: &[MarketDataBar], params: &Value) -> Result<Vec<Value>, Error> {
    let p: HmaParams = parse_params(params)?;
    rows_scalar(&moving_average::hma(&closes(bars), p).map_err(|e| map_period_err(&e))?)
}

fn compute_macd(bars: &[MarketDataBar], params: &Value) -> Result<Vec<Value>, Error> {
    let p: MacdParams = parse_params(params)?;
    rows_struct(&moving_average::macd(&closes(bars), p).map_err(|e| map_period_err(&e))?)
}

fn compute_rsi(bars: &[MarketDataBar], params: &Value) -> Result<Vec<Value>, Error> {
    let p: LengthParams = parse_params(params)?;
    rows_scalar(&oscillators::rsi(&closes(bars), p).map_err(|e| map_period_err(&e))?)
}

fn compute_stochastic(bars: &[MarketDataBar], params: &Value) -> Result<Vec<Value>, Error> {
    let p: StochasticParams = parse_params(params)?;
    rows_struct(&oscillators::stochastic(bars, p).map_err(|e| map_period_err(&e))?)
}

fn compute_cci(bars: &[MarketDataBar], params: &Value) -> Result<Vec<Value>, Error> {
    let p: CciParams = parse_params(params)?;
    rows_scalar(&oscillators::cci(bars, p).map_err(|e| map_period_err(&e))?)
}

fn compute_adx(bars: &[MarketDataBar], params: &Value) -> Result<Vec<Value>, Error> {
    let p: AdxParams = parse_params(params)?;
    rows_struct(&trend::adx(bars, p).map_err(|e| map_period_err(&e))?)
}

fn compute_aroon(bars: &[MarketDataBar], params: &Value) -> Result<Vec<Value>, Error> {
    let p: AroonParams = parse_params(params)?;
    rows_struct(&trend::aroon(bars, p).map_err(|e| map_period_err(&e))?)
}

fn compute_bbands(bars: &[MarketDataBar], params: &Value) -> Result<Vec<Value>, Error> {
    let p: BollingerParams = parse_params(params)?;
    rows_struct(&bands::bollinger(bars, p).map_err(|e| map_period_err(&e))?)
}

fn compute_keltner(bars: &[MarketDataBar], params: &Value) -> Result<Vec<Value>, Error> {
    let p: KeltnerParams = parse_params(params)?;
    rows_struct(&bands::keltner(bars, p).map_err(|e| map_period_err(&e))?)
}

fn compute_donchian(bars: &[MarketDataBar], params: &Value) -> Result<Vec<Value>, Error> {
    let p: DonchianParams = parse_params(params)?;
    rows_struct(&bands::donchian(bars, p).map_err(|e| map_period_err(&e))?)
}

fn compute_atr(bars: &[MarketDataBar], params: &Value) -> Result<Vec<Value>, Error> {
    let p: LengthParams = parse_params(params)?;
    rows_scalar(&volume::atr(bars, p).map_err(|e| map_period_err(&e))?)
}

fn compute_obv(bars: &[MarketDataBar], _params: &Value) -> Result<Vec<Value>, Error> {
    rows_scalar(&volume::obv(bars))
}

fn compute_ad(bars: &[MarketDataBar], _params: &Value) -> Result<Vec<Value>, Error> {
    rows_scalar(&volume::ad(bars))
}

fn compute_vwap(bars: &[MarketDataBar], _params: &Value) -> Result<Vec<Value>, Error> {
    rows_scalar(&volume::vwap(bars))
}

fn compute_fisher(bars: &[MarketDataBar], params: &Value) -> Result<Vec<Value>, Error> {
    let p: FisherParams = parse_params(params)?;
    rows_struct(&oscillators::fisher(bars, p).map_err(|e| map_period_err(&e))?)
}

fn compute_roc(bars: &[MarketDataBar], params: &Value) -> Result<Vec<Value>, Error> {
    let p: RocParams = parse_params(params)?;
    rows_scalar(&oscillators::roc(&closes(bars), p).map_err(|e| map_period_err(&e))?)
}

fn compute_momentum(bars: &[MarketDataBar], params: &Value) -> Result<Vec<Value>, Error> {
    let p: RocParams = parse_params(params)?;
    rows_scalar(&oscillators::momentum(&closes(bars), p).map_err(|e| map_period_err(&e))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_records() -> Value {
        json!([
            { "date": "2026-01-01", "open": 10.0, "high": 10.5, "low": 9.8, "close": 10.0, "volume": 1000 },
            { "date": "2026-01-02", "open": 10.0, "high": 11.2, "low": 9.9, "close": 11.0, "volume": 1100 },
            { "date": "2026-01-03", "open": 11.0, "high": 12.4, "low": 10.8, "close": 12.0, "volume": 1200 }
        ])
    }

    #[test]
    fn parse_bars_accepts_date_and_integer_volume() {
        let bars = parse_bars(&sample_records()).expect("parse");
        assert_eq!(bars.len(), 3);
        assert_eq!(bars[0].ts, "2026-01-01");
        assert!((bars[2].close - 12.0).abs() < 1e-12);
    }

    #[test]
    fn run_compute_sma_is_date_aligned() {
        let bars = parse_bars(&sample_records()).expect("parse");
        let rows = run_compute("technical/sma", &bars, &json!({ "length": 2 })).expect("sma");
        assert_eq!(rows.len(), bars.len());
        assert_eq!(rows[0], Value::Null);
        assert!((rows[1].as_f64().expect("f64") - 10.5).abs() < 1e-12);
    }

    #[test]
    fn run_compute_unknown_route_errors() {
        let bars = parse_bars(&sample_records()).expect("parse");
        assert!(run_compute("technical/nope", &bars, &json!({})).is_err());
    }

    #[test]
    fn run_compute_zero_period_is_invalid_query() {
        let bars = parse_bars(&sample_records()).expect("parse");
        let err = run_compute("technical/sma", &bars, &json!({ "length": 0 })).expect_err("err");
        assert!(matches!(err, Error::InvalidQuery(_)), "got {err:?}");
    }

    #[test]
    fn registry_routes_match_catalog_compute_routes() {
        use std::collections::BTreeSet;
        let registry: BTreeSet<&str> = compute_registry().keys().copied().collect();
        let catalog: BTreeSet<&str> = tdw_endpoint_catalog::catalog()
            .iter()
            .filter(|e| e.kind == tdw_endpoint_catalog::EndpointKind::Compute)
            .map(|e| e.route)
            .collect();
        assert_eq!(
            registry, catalog,
            "compute registry must match catalog Compute routes"
        );
    }
}
