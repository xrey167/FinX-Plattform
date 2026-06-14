#![cfg(feature = "http")]
//! Real CBOE HTTP fetchers for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to CBOE's public CDN endpoints directly
//! via `reqwest`. No API key is required. Live calls are additionally gated by
//! `TDW_CBOE_LIVE=1` so unattended CI stays offline.

use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Error, Result};
use tdw_provider_http::{HttpFetcher, ProviderSpec};

use crate::{
    BASE_URL, CboeIndexDirectoryQuery, CboeIndexQuery, CboeOptionsQuery, CboeProviderError,
    index_directory_path, index_request_path, options_request_path,
};

const USER_AGENT: &str = "tdw-provider-cboe/0.1";

// ---------------------------------------------------------------------------
// Serde response shapes
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct OptionsEnvelope {
    data: OptionsData,
}

#[derive(Deserialize)]
struct OptionsData {
    options: Vec<RawOptionContract>,
}

#[derive(Deserialize)]
struct RawOptionContract {
    option: String,
    bid: f64,
    ask: f64,
    iv: f64,
    delta: f64,
    gamma: f64,
    theta: f64,
    open_interest: u64,
}

#[derive(Deserialize)]
struct IndexEnvelope {
    data: IndexData,
}

#[derive(Deserialize)]
struct IndexData {
    symbol: String,
    price: f64,
    change: f64,
    volume: u64,
}

// ---------------------------------------------------------------------------
// Domain output types (re-exported from lib.rs)
// ---------------------------------------------------------------------------

use crate::{CboeIndexQuote, CboeOptionContract};

// ---------------------------------------------------------------------------
// Options fetcher
// ---------------------------------------------------------------------------

/// Provider specification for the CBOE delayed options chain fetcher.
pub struct CboeOptionsSpec;

impl ProviderSpec for CboeOptionsSpec {
    const PROVIDER: &'static str = "cboe";
    const ENDPOINT: &'static str = "options";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "cboe options client build";
    const SEND_ERR: &'static str = "cboe options request";
    const RETURNED_ERR: &'static str = "cboe options returned";
    const READ_BODY_ERR: &'static str = "cboe options read body";

    type Query = CboeOptionsQuery;
    type Data = CboeOptionContract;

    fn transform_query(params: Value) -> Result<CboeOptionsQuery> {
        let symbol = params
            .get("symbol")
            .or_else(|| params.get("ticker"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("cboe options symbol must be a string".to_string())
            })?;
        CboeOptionsQuery::new(symbol).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    fn build_request(
        base_url: &str,
        query: &CboeOptionsQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        let path =
            options_request_path(&query.symbol).map_err(|e| Error::Provider(e.to_string()))?;
        let url = format!("{}{path}", base_url.trim_end_matches('/'));
        Ok(client.get(&url))
    }

    fn transform_data(_query: &CboeOptionsQuery, raw: Bytes) -> Result<Vec<CboeOptionContract>> {
        let envelope: OptionsEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("cboe options parse_json: {e}")))?;

        let contracts = envelope
            .data
            .options
            .into_iter()
            .map(|r| CboeOptionContract {
                option: r.option,
                bid: r.bid,
                ask: r.ask,
                iv: r.iv,
                delta: r.delta,
                gamma: r.gamma,
                theta: r.theta,
                open_interest: r.open_interest,
            })
            .collect();

        Ok(contracts)
    }
}

/// Production CBOE delayed options chain fetcher.
pub type CboeHttpOptionsFetcher = HttpFetcher<CboeOptionsSpec>;

// ---------------------------------------------------------------------------
// Index fetcher
// ---------------------------------------------------------------------------

/// Provider specification for the CBOE US-index quote fetcher.
pub struct CboeIndexSpec;

impl ProviderSpec for CboeIndexSpec {
    const PROVIDER: &'static str = "cboe";
    const ENDPOINT: &'static str = "index_quotes";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "cboe index client build";
    const SEND_ERR: &'static str = "cboe index request";
    const RETURNED_ERR: &'static str = "cboe index returned";
    const READ_BODY_ERR: &'static str = "cboe index read body";

    type Query = CboeIndexQuery;
    type Data = CboeIndexQuote;

    fn transform_query(params: Value) -> Result<CboeIndexQuery> {
        let index = params
            .get("index")
            .or_else(|| params.get("symbol"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("cboe index must be a string".to_string()))?;
        CboeIndexQuery::new(index).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    fn build_request(
        base_url: &str,
        query: &CboeIndexQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        let path = index_request_path(&query.index).map_err(|e| Error::Provider(e.to_string()))?;
        let url = format!("{}{path}", base_url.trim_end_matches('/'));
        Ok(client.get(&url))
    }

    fn transform_data(_query: &CboeIndexQuery, raw: Bytes) -> Result<Vec<CboeIndexQuote>> {
        let envelope: IndexEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("cboe index parse_json: {e}")))?;

        Ok(vec![CboeIndexQuote {
            symbol: envelope.data.symbol,
            price: envelope.data.price,
            change: envelope.data.change,
            volume: envelope.data.volume,
        }])
    }
}

/// Production CBOE US-index quote fetcher.
pub type CboeHttpIndexFetcher = HttpFetcher<CboeIndexSpec>;

// ---------------------------------------------------------------------------
// Catalog-facing fetchers -> tdw_domain models
//
// The bespoke `CboeHttpIndexFetcher`/`CboeHttpOptionsFetcher` above emit the
// crate-local `CboeIndexQuote`/`CboeOptionContract`. The namespaced endpoint
// catalog routes (`index/snapshots`, `derivatives/options/chains`) need
// fetchers that emit `tdw_domain` models. These thin wrappers reuse the
// existing `ProviderSpec` request builders and JSON parsers verbatim (no
// parsing logic is duplicated), then map onto the standardized shapes.
//
// CBOE delayed US-index quotes:
// <https://cdn.cboe.com/api/global/us_indices/quotes/{index}.json>.
// CBOE delayed options chains:
// <https://cdn.cboe.com/api/global/delayed_quotes/options/{symbol}.json>.
// ---------------------------------------------------------------------------

use async_trait::async_trait;
use tdw_core::{Credentials, Fetcher};
use tdw_domain::{Instrument, OptionContract, QuoteSnapshot};

/// Send a request built by `P` and return the response bytes, reusing the
/// shared bounded client. Mirrors the blanket `HttpFetcher` extract path so the
/// catalog wrappers issue byte-identical requests without re-inlining I/O.
async fn cboe_fetch<P: ProviderSpec>(base_url: &str, query: &P::Query) -> Result<Bytes> {
    let client = tdw_core::http_support::build_client(P::USER_AGENT, P::CLIENT_ERR)?;
    let request = P::build_request(base_url, query, &client)?;
    let response = request
        .send()
        .await
        .map_err(|e| Error::Provider(format!("{}: {e}", P::SEND_ERR)))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Provider(format!(
            "{} {status}: {body}",
            P::RETURNED_ERR
        )));
    }
    response
        .bytes()
        .await
        .map_err(|e| Error::Provider(format!("{}: {e}", P::READ_BODY_ERR)))
}

tdw_core::provider_fetcher_struct!(
    /// Production CBOE delayed US-index quote fetcher (catalog-facing).
    ///
    /// Standardizes `index/snapshots`: the delayed US-index quote, normalized to
    /// [`tdw_domain::QuoteSnapshot`]. Reuses [`CboeIndexSpec`]'s request builder
    /// and parser; `change_percent` is derived from the quote's absolute change
    /// and the implied previous close.
    pub CboeHttpIndexSnapshotFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<CboeIndexQuery, QuoteSnapshot> for CboeHttpIndexSnapshotFetcher {
    const PROVIDER: &'static str = "cboe";
    const ENDPOINT: &'static str = "index_snapshots";

    fn transform_query(params: Value) -> Result<CboeIndexQuery> {
        <CboeIndexSpec as ProviderSpec>::transform_query(params)
    }

    async fn extract_data(&self, query: &CboeIndexQuery, _creds: &Credentials) -> Result<Bytes> {
        cboe_fetch::<CboeIndexSpec>(self.base_url(), query).await
    }

    fn transform_data(&self, query: &CboeIndexQuery, raw: Bytes) -> Result<Vec<QuoteSnapshot>> {
        let quotes = <CboeIndexSpec as ProviderSpec>::transform_data(query, raw)?;
        Ok(quotes
            .into_iter()
            .map(|quote| {
                let prev_close = quote.price - quote.change;
                let change_percent = if prev_close == 0.0 {
                    0.0
                } else {
                    quote.change / prev_close * 100.0
                };
                QuoteSnapshot {
                    symbol: quote.symbol,
                    current_price: quote.price,
                    change: quote.change,
                    change_percent,
                    prev_close,
                    // The delayed index feed carries no quote timestamp; 0 marks
                    // "unknown epoch" (the price-alert read path treats ts_ms as
                    // advisory).
                    ts_ms: 0,
                }
            })
            .collect())
    }
}

tdw_core::provider_fetcher_struct!(
    /// Production CBOE delayed options-chain fetcher (catalog-facing).
    ///
    /// Standardizes `derivatives/options/chains`: the delayed options chain,
    /// normalized to [`tdw_domain::OptionContract`]. Reuses [`CboeOptionsSpec`]'s
    /// request builder and parser; the option type and expiry are decoded from
    /// the CBOE OCC-style contract symbol (`{ROOT}{YYMMDD}{C|P}{STRIKE*1000}`).
    pub CboeHttpOptionsChainFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<CboeOptionsQuery, OptionContract> for CboeHttpOptionsChainFetcher {
    const PROVIDER: &'static str = "cboe";
    const ENDPOINT: &'static str = "options_chains";

    fn transform_query(params: Value) -> Result<CboeOptionsQuery> {
        <CboeOptionsSpec as ProviderSpec>::transform_query(params)
    }

    async fn extract_data(&self, query: &CboeOptionsQuery, _creds: &Credentials) -> Result<Bytes> {
        cboe_fetch::<CboeOptionsSpec>(self.base_url(), query).await
    }

    fn transform_data(&self, query: &CboeOptionsQuery, raw: Bytes) -> Result<Vec<OptionContract>> {
        let contracts = <CboeOptionsSpec as ProviderSpec>::transform_data(query, raw)?;
        Ok(contracts
            .into_iter()
            .map(|contract| {
                let (expiration, option_type, strike) = decode_occ_symbol(&contract.option);
                OptionContract {
                    underlying_symbol: query.symbol.clone(),
                    contract_symbol: Some(contract.option),
                    expiration,
                    strike,
                    option_type,
                    bid: Some(contract.bid),
                    ask: Some(contract.ask),
                    last_price: None,
                    volume: None,
                    open_interest: Some(contract.open_interest),
                    implied_volatility: Some(contract.iv),
                    delta: Some(contract.delta),
                    gamma: Some(contract.gamma),
                    theta: Some(contract.theta),
                    vega: None,
                    rho: None,
                }
            })
            .collect())
    }
}

/// Decode an OCC-style CBOE contract symbol into `(expiration, option_type,
/// strike)`.
///
/// The CBOE delayed-chain `option` field is the OCC identifier
/// `{ROOT}{YYMMDD}{C|P}{STRIKE × 1000, 8 digits}` (e.g.
/// `AAPL240119C00180000`). The root may contain digits, so the date/type/strike
/// tail is parsed from the right (a fixed 15-character suffix). On any shape the
/// parser does not recognise it returns conservative fallbacks so a malformed
/// row never aborts the whole chain.
fn decode_occ_symbol(symbol: &str) -> (String, String, f64) {
    const TAIL_LEN: usize = 15; // 6 date + 1 type + 8 strike
    let bytes = symbol.as_bytes();
    if bytes.len() < TAIL_LEN {
        return (String::new(), "call".to_string(), 0.0);
    }
    let tail = &symbol[symbol.len() - TAIL_LEN..];
    let date = &tail[0..6];
    let type_char = &tail[6..7];
    let strike_raw = &tail[7..15];

    let expiration = if date.bytes().all(|b| b.is_ascii_digit()) {
        format!("20{}-{}-{}", &date[0..2], &date[2..4], &date[4..6])
    } else {
        String::new()
    };
    let option_type = if type_char.eq_ignore_ascii_case("P") {
        "put".to_string()
    } else {
        "call".to_string()
    };
    let strike = strike_raw
        .parse::<f64>()
        .map_or(0.0, |thousandths| thousandths / 1000.0);
    (expiration, option_type, strike)
}

// ---------------------------------------------------------------------------
// CBOE index directory -> tdw_domain::Instrument
//
// The keyless CDN document
// <https://cdn.cboe.com/api/global/us_indices/definitions/all_us_indices.json>
// lists every CBOE index (name, symbol, description). One fetcher serves both
// `index/available` (full listing) and `index/search` (case-insensitive
// substring filter over symbol/name), normalized to `tdw_domain::Instrument`.
// ---------------------------------------------------------------------------

/// Wire shape for one entry in the CBOE index-definitions document.
#[derive(Deserialize)]
struct RawIndexDefinition {
    #[serde(default)]
    index: String,
    #[serde(default)]
    name: String,
}

tdw_core::provider_fetcher_struct!(
    /// Production CBOE index-directory fetcher (catalog-facing).
    ///
    /// Standardizes `index/available` and `index/search`: the CBOE index
    /// definitions document, normalized to [`tdw_domain::Instrument`]
    /// (`symbol`/`name`/`venue = "cboe"`). The optional `query` param applies a
    /// case-insensitive substring filter over the symbol and name; an empty
    /// query lists every index.
    pub CboeHttpIndexDirectoryFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<CboeIndexDirectoryQuery, Instrument> for CboeHttpIndexDirectoryFetcher {
    const PROVIDER: &'static str = "cboe";
    const ENDPOINT: &'static str = "index_directory";

    fn transform_query(params: Value) -> Result<CboeIndexDirectoryQuery> {
        let query = params
            .get("query")
            .or_else(|| params.get("q"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        Ok(CboeIndexDirectoryQuery::new(query))
    }

    async fn extract_data(
        &self,
        _query: &CboeIndexDirectoryQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let client =
            tdw_core::http_support::build_client(USER_AGENT, "cboe directory client build")?;
        let url = format!(
            "{}{}",
            self.base_url().trim_end_matches('/'),
            index_directory_path()
        );
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("cboe directory request: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "cboe directory returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("cboe directory read body: {e}")))
    }

    fn transform_data(
        &self,
        query: &CboeIndexDirectoryQuery,
        raw: Bytes,
    ) -> Result<Vec<Instrument>> {
        let entries: Vec<RawIndexDefinition> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("cboe directory parse_json: {e}")))?;
        let needle = query.query.to_ascii_lowercase();
        let instruments = entries
            .into_iter()
            .filter_map(|entry| {
                let symbol = entry.index.trim().to_string();
                if symbol.is_empty() {
                    return None;
                }
                let name = if entry.name.trim().is_empty() {
                    symbol.clone()
                } else {
                    entry.name.trim().to_string()
                };
                if !needle.is_empty()
                    && !symbol.to_ascii_lowercase().contains(&needle)
                    && !name.to_ascii_lowercase().contains(&needle)
                {
                    return None;
                }
                Some(Instrument {
                    symbol,
                    name,
                    venue: "cboe".to_string(),
                })
            })
            .collect();
        Ok(instruments)
    }
}

// ---------------------------------------------------------------------------
// Error mapping helper (unused directly but kept for completeness)
// ---------------------------------------------------------------------------

impl From<CboeProviderError> for Error {
    fn from(e: CboeProviderError) -> Self {
        Self::Provider(e.to_string())
    }
}
