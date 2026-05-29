#![forbid(unsafe_code)]

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

/// Subscription query for the generic websocket tick streamer.
///
/// `url` is the websocket endpoint (`ws://` / `wss://`) the live `ws`-feature
/// path connects to; `symbol` is echoed into the deterministic offline
/// `snapshot`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WsTickQuery {
    pub url: String,
    pub symbol: String,
}

#[derive(Clone, Debug, Default)]
pub struct WsTickStreamer;

impl WsTickStreamer {
    #[must_use]
    pub const fn registry_entry() -> RegistryEntry {
        RegistryEntry::streamer(Self::PROVIDER, Self::ENDPOINT)
    }
}

/// Decode a text websocket frame into ticks.
///
/// Accepts either a JSON array of tick objects (`[{...}, {...}]`) or
/// newline-delimited JSON (one tick object per line). Blank lines are ignored.
/// This function is pure and offline: it performs no IO, so it is the unit-test
/// seam for the live `subscribe` path.
///
/// # Errors
///
/// Returns [`Error::Provider`] if the frame is neither a valid JSON array of
/// ticks nor valid NDJSON of ticks.
pub fn decode_frame(text: &str) -> Result<Vec<Tick>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if trimmed.starts_with('[') {
        return serde_json::from_str::<Vec<Tick>>(trimmed)
            .map_err(|error| Error::Provider(format!("ws decode array frame: {error}")));
    }
    let mut ticks = Vec::new();
    for line in trimmed.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let tick = serde_json::from_str::<Tick>(line)
            .map_err(|error| Error::Provider(format!("ws decode ndjson frame: {error}")))?;
        ticks.push(tick);
    }
    Ok(ticks)
}

fn snapshot_rows(symbol: &str) -> Vec<Tick> {
    vec![Tick {
        symbol: symbol.to_string(),
        venue: "WS".to_string(),
        ts: "2026-05-21T20:00:00Z".to_string(),
        price: 100.0,
        size: 1.0,
    }]
}

#[async_trait]
impl Streamer<WsTickQuery, Tick> for WsTickStreamer {
    const PROVIDER: &'static str = "ws";
    const ENDPOINT: &'static str = "equity_ticks";

    #[cfg(feature = "ws")]
    async fn subscribe(
        &self,
        query: WsTickQuery,
        _creds: &Credentials,
    ) -> Result<DataStream<Tick>> {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::Message;

        let (socket, _response) = tokio_tungstenite::connect_async(&query.url)
            .await
            .map_err(|error| Error::Provider(format!("ws connect {}: {error}", query.url)))?;
        let frames = socket.filter_map(|message| async move {
            match message {
                Ok(Message::Text(text)) => Some(decode_frame(text.as_str())),
                Ok(_) => None,
                Err(error) => Some(Err(Error::Provider(format!("ws frame: {error}")))),
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
        query: WsTickQuery,
        _creds: &Credentials,
    ) -> Result<DataStream<Tick>> {
        Ok(Box::pin(VecStream::new(snapshot_rows(&query.symbol))))
    }

    async fn snapshot(&self, query: &WsTickQuery, _creds: &Credentials) -> Result<Vec<Tick>> {
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
        let entry = WsTickStreamer::registry_entry();

        assert_eq!(entry.provider, "ws");
        assert_eq!(entry.endpoint, "equity_ticks");
    }

    #[test]
    fn decode_frame_parses_json_array() {
        let frame = r#"[
            {"symbol":"AAPL","venue":"WS","ts":"2026-05-21T20:00:00Z","price":100.5,"size":10.0},
            {"symbol":"MSFT","venue":"WS","ts":"2026-05-21T20:00:01Z","price":420.0,"size":2.0}
        ]"#;
        let ticks = decode_frame(frame)
            .unwrap_or_else(|error| panic!("array frame should decode: {error}"));

        assert_eq!(ticks.len(), 2);
        assert_eq!(ticks[0].symbol, "AAPL");
        assert_eq!(ticks[1].symbol, "MSFT");
    }

    #[test]
    fn decode_frame_parses_ndjson_and_skips_blank_lines() {
        let frame = "\n{\"symbol\":\"AAPL\",\"venue\":\"WS\",\"ts\":\"2026-05-21T20:00:00Z\",\"price\":100.5,\"size\":10.0}\n\n{\"symbol\":\"MSFT\",\"venue\":\"WS\",\"ts\":\"2026-05-21T20:00:01Z\",\"price\":420.0,\"size\":2.0}\n";
        let ticks = decode_frame(frame)
            .unwrap_or_else(|error| panic!("ndjson frame should decode: {error}"));

        assert_eq!(ticks.len(), 2);
        assert_eq!(ticks[1].symbol, "MSFT");
    }

    #[test]
    fn decode_frame_handles_empty_input() {
        let ticks = decode_frame("   \n  ")
            .unwrap_or_else(|error| panic!("empty frame should decode to no ticks: {error}"));

        assert!(ticks.is_empty());
    }

    #[test]
    fn decode_frame_rejects_invalid_payload() {
        assert!(decode_frame("not json").is_err());
        assert!(decode_frame("[{\"symbol\":\"AAPL\"}]").is_err());
        assert!(decode_frame("{\"symbol\":\"AAPL\"}").is_err());
    }
}
