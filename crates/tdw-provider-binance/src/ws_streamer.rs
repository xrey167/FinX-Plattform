//! Binance trade websocket streamer.
//!
//! Mirrors the structure of `tdw-provider-ws`'s generic `WsTickStreamer`: a
//! `ws`-feature-gated live subscribe path (`tokio-tungstenite`), a pure offline
//! `decode_trade_frame` unit-test seam, a deterministic offline subscribe, and a
//! `snapshot`.

#[cfg(not(feature = "ws"))]
use std::collections::VecDeque;
#[cfg(not(feature = "ws"))]
use std::pin::Pin;
#[cfg(not(feature = "ws"))]
use std::task::{Context, Poll};

use async_trait::async_trait;
#[cfg(not(feature = "ws"))]
use futures_core::Stream;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tdw_core::{Credentials, DataStream, Error, RegistryEntry, Result, Streamer};
use tdw_domain::Tick;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::normalize_symbol;

/// Venue label stamped on every decoded Binance trade tick.
const VENUE: &str = "BINANCE";

/// Subscription query for the Binance trade websocket streamer.
///
/// `symbol` is a normalized Binance market symbol (e.g. `BTCUSDT`). The live
/// `ws`-feature subscribe path lowercases it into the
/// `wss://stream.binance.com:9443/ws/{symbol}@trade` endpoint; the deterministic
/// offline `snapshot` echoes it back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BinanceTradeQuery {
    pub symbol: String,
}

impl BinanceTradeQuery {
    /// Build a [`BinanceTradeQuery`] from a raw symbol, normalizing it via
    /// [`normalize_symbol`].
    ///
    /// # Errors
    ///
    /// Returns [`crate::BinanceProviderError`] if the symbol is empty or
    /// contains characters other than ASCII alphanumerics.
    pub fn new(symbol: &str) -> crate::Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
        })
    }
}

/// Live Binance trade websocket streamer.
///
/// Implements [`Streamer`] for [`BinanceTradeQuery`] yielding [`Tick`]s. With
/// the `ws` feature it connects to the public Binance trade stream; without it,
/// the subscribe path returns a deterministic single-tick stream so the
/// workspace test set stays offline.
#[derive(Clone, Debug, Default)]
pub struct BinanceTradeStreamer;

impl BinanceTradeStreamer {
    /// Registry entry describing this streamer (`binance`/`trades`).
    #[must_use]
    pub const fn registry_entry() -> RegistryEntry {
        RegistryEntry::streamer(Self::PROVIDER, Self::ENDPOINT)
    }
}

/// Convert a Binance epoch-millisecond trade timestamp into an RFC3339 string.
///
/// # Errors
///
/// Returns [`Error::Provider`] if the millisecond value is out of range for a
/// representable timestamp or if RFC3339 formatting fails.
fn millis_to_rfc3339(millis: i64) -> Result<String> {
    let nanos = i128::from(millis) * 1_000_000;
    let datetime = OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .map_err(|error| Error::Provider(format!("binance trade timestamp {millis}: {error}")))?;
    datetime
        .format(&Rfc3339)
        .map_err(|error| Error::Provider(format!("binance trade timestamp format: {error}")))
}

/// Decode a Binance `@trade` text websocket frame into ticks.
///
/// Accepts both the single-stream bare trade object
/// (`{"e":"trade","s":"BTCUSDT","p":"68000.50","q":"0.01","T":1700000000000}`)
/// and the combined-stream wrapper
/// (`{"stream":"btcusdt@trade","data":{...trade...}}`). The mapped tick is:
/// `symbol` = uppercased `s`, `venue` = `"BINANCE"`, `ts` = RFC3339 from the `T`
/// epoch-millisecond field, `price` = parsed `p`, `size` = parsed `q`.
///
/// Frames that are not `trade` events (subscription acks, ping/pong results such
/// as `{"result":null,"id":1}`) and blank input decode to `Ok(vec![])` — they
/// are ignored gracefully rather than treated as errors. This function is pure
/// and offline (no IO), so it is the unit-test seam for the live `subscribe`
/// path.
///
/// # Errors
///
/// Returns [`Error::Provider`] if the frame is not valid JSON, or if a `trade`
/// event is missing required fields or carries an unparseable price/size.
pub fn decode_trade_frame(text: &str) -> Result<Vec<Tick>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let value: serde_json::Value = serde_json::from_str(trimmed)
        .map_err(|error| Error::Provider(format!("binance trade decode frame: {error}")))?;

    // Unwrap the combined-stream wrapper `{"stream":..,"data":{trade}}` if present.
    let trade = value.get("data").unwrap_or(&value);

    // Ignore non-trade events (acks, ping results, other stream payloads).
    if trade.get("e").and_then(serde_json::Value::as_str) != Some("trade") {
        return Ok(Vec::new());
    }

    let symbol = trade
        .get("s")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| Error::Provider("binance trade missing symbol (s)".to_string()))?
        .to_ascii_uppercase();
    let price = parse_decimal(trade.get("p"), "price (p)")?;
    let size = parse_decimal(trade.get("q"), "size (q)")?;
    let millis = trade
        .get("T")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| Error::Provider("binance trade missing trade time (T)".to_string()))?;
    let ts = millis_to_rfc3339(millis)?;

    Ok(vec![Tick {
        symbol,
        venue: VENUE.to_string(),
        ts,
        price,
        size,
    }])
}

/// Parse a Binance string-encoded decimal field (`"68000.50"`) into an `f64`.
fn parse_decimal(value: Option<&serde_json::Value>, field: &str) -> Result<f64> {
    let raw = value
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| Error::Provider(format!("binance trade missing {field}")))?;
    raw.parse::<f64>()
        .map_err(|error| Error::Provider(format!("binance trade {field} parse: {error}")))
}

fn snapshot_rows(symbol: &str) -> Vec<Tick> {
    vec![Tick {
        symbol: symbol.to_ascii_uppercase(),
        venue: VENUE.to_string(),
        ts: "2026-05-21T20:00:00Z".to_string(),
        price: 68_000.0,
        size: 0.01,
    }]
}

#[async_trait]
impl Streamer<BinanceTradeQuery, Tick> for BinanceTradeStreamer {
    const PROVIDER: &'static str = "binance";
    const ENDPOINT: &'static str = "trades";

    #[cfg(feature = "ws")]
    async fn subscribe(
        &self,
        query: BinanceTradeQuery,
        _creds: &Credentials,
    ) -> Result<DataStream<Tick>> {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::Message;

        let url = format!(
            "wss://stream.binance.com:9443/ws/{}@trade",
            query.symbol.to_ascii_lowercase()
        );
        let (socket, _response) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(|error| Error::Provider(format!("binance ws connect {url}: {error}")))?;
        let frames = socket.filter_map(|message| async move {
            match message {
                Ok(Message::Text(text)) => Some(decode_trade_frame(text.as_str())),
                Ok(_) => None,
                Err(error) => Some(Err(Error::Provider(format!("binance ws frame: {error}")))),
            }
        });
        let ticks = frames.flat_map(|result| {
            let items: Vec<Result<Tick>> = match result {
                Ok(ticks) => ticks.into_iter().map(Ok).collect(),
                Err(error) => vec![Err(error)],
            };
            futures_util::stream::iter(items)
        });
        Ok(Box::pin(ticks))
    }

    #[cfg(not(feature = "ws"))]
    async fn subscribe(
        &self,
        query: BinanceTradeQuery,
        _creds: &Credentials,
    ) -> Result<DataStream<Tick>> {
        Ok(Box::pin(VecStream::new(snapshot_rows(&query.symbol))))
    }

    async fn snapshot(&self, query: &BinanceTradeQuery, _creds: &Credentials) -> Result<Vec<Tick>> {
        Ok(snapshot_rows(&query.symbol))
    }
}

#[cfg(not(feature = "ws"))]
struct VecStream<T> {
    items: VecDeque<Result<T>>,
}

#[cfg(not(feature = "ws"))]
impl<T> VecStream<T> {
    fn new(items: Vec<T>) -> Self {
        Self {
            items: items.into_iter().map(Ok).collect(),
        }
    }
}

#[cfg(not(feature = "ws"))]
impl<T> Unpin for VecStream<T> {}

#[cfg(not(feature = "ws"))]
impl<T> Stream for VecStream<T> {
    type Item = Result<T>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.get_mut().items.pop_front())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_streamer_registration() {
        let entry = BinanceTradeStreamer::registry_entry();

        assert_eq!(entry.provider, "binance");
        assert_eq!(entry.endpoint, "trades");
    }

    #[test]
    fn builds_trade_query_model() {
        let query = BinanceTradeQuery::new("btcusdt")
            .unwrap_or_else(|error| panic!("query should build: {error}"));

        assert_eq!(query.symbol, "BTCUSDT");
        assert!(BinanceTradeQuery::new("").is_err());
        assert!(BinanceTradeQuery::new("BTCUSDT@trade").is_err());
    }

    #[test]
    fn decode_single_stream_trade_frame() {
        let frame = r#"{"e":"trade","E":1700000000123,"s":"btcusdt","t":12345,"p":"68000.50","q":"0.01","T":1700000000000,"m":false}"#;
        let ticks = decode_trade_frame(frame)
            .unwrap_or_else(|error| panic!("trade frame should decode: {error}"));

        assert_eq!(ticks.len(), 1);
        let tick = &ticks[0];
        assert_eq!(tick.symbol, "BTCUSDT");
        assert_eq!(tick.venue, "BINANCE");
        assert!((tick.price - 68_000.50).abs() < f64::EPSILON);
        assert!((tick.size - 0.01).abs() < f64::EPSILON);
        // ts must be a valid RFC3339 timestamp (1700000000000 ms = 2023-11-14T22:13:20Z).
        OffsetDateTime::parse(&tick.ts, &Rfc3339)
            .unwrap_or_else(|error| panic!("ts should be RFC3339: {error}"));
        assert_eq!(tick.ts, "2023-11-14T22:13:20Z");
    }

    #[test]
    fn decode_combined_stream_wrapper() {
        let frame = r#"{"stream":"btcusdt@trade","data":{"e":"trade","s":"ETHUSDT","p":"3200.10","q":"2.5","T":1700000000000}}"#;
        let ticks = decode_trade_frame(frame)
            .unwrap_or_else(|error| panic!("combined frame should decode: {error}"));

        assert_eq!(ticks.len(), 1);
        assert_eq!(ticks[0].symbol, "ETHUSDT");
        assert!((ticks[0].price - 3200.10).abs() < f64::EPSILON);
        assert!((ticks[0].size - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn decode_non_trade_event_is_ignored() {
        assert!(
            decode_trade_frame(r#"{"result":null,"id":1}"#)
                .unwrap_or_else(|error| panic!("ack frame should decode to no ticks: {error}"))
                .is_empty()
        );
        assert!(
            decode_trade_frame(r#"{"e":"depthUpdate","s":"BTCUSDT"}"#)
                .unwrap_or_else(|error| panic!(
                    "non-trade event should decode to no ticks: {error}"
                ))
                .is_empty()
        );
    }

    #[test]
    fn decode_blank_input_is_empty() {
        assert!(
            decode_trade_frame("   \n  ")
                .unwrap_or_else(|error| panic!("blank frame should decode to no ticks: {error}"))
                .is_empty()
        );
    }

    #[test]
    fn decode_malformed_price_is_error() {
        let frame =
            r#"{"e":"trade","s":"BTCUSDT","p":"not-a-number","q":"0.01","T":1700000000000}"#;
        assert!(decode_trade_frame(frame).is_err());
    }

    #[test]
    fn decode_invalid_json_is_error() {
        assert!(decode_trade_frame("not json").is_err());
    }
}
