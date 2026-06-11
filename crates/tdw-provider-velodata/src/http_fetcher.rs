//! Real Velodata HTTP fetchers for funding rates, liquidations, and open
//! interest.
//!
//! Gated by the `http` feature — reqwest is not pulled in otherwise.
//! Live integration tests are additionally gated by `TDW_VELODATA_LIVE=1`
//! so unattended CI stays offline.

#![cfg(feature = "http")]

use tdw_core::http_support::prelude::*;

use crate::{
    API_KEY_ENV, API_KEY_HEADER, AggregatedLiquidation, AggregatedOi, BASE_URL, FundingRate,
    VelodataFundingQuery, VelodataLiquidationsQuery, VelodataOiQuery, funding_query_from_value,
    liquidations_query_from_value, oi_query_from_value,
};

const USER_AGENT: &str = "tdw-provider-velodata/0.1";

// ---------------------------------------------------------------------------
// Shared HTTP helper
// ---------------------------------------------------------------------------

async fn get_bytes(url: &str, api_key: &str) -> Result<Bytes> {
    let client = Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| Error::Provider(format!("velodata: build client: {e}")))?;
    let response = client
        .get(url)
        .header(API_KEY_HEADER, api_key)
        .send()
        .await
        .map_err(|e| Error::Provider(format!("velodata: request failed: {e}")))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Provider(format!("velodata: {status}: {body}")));
    }
    response
        .bytes()
        .await
        .map_err(|e| Error::Provider(format!("velodata: read body: {e}")))
}

fn read_api_key() -> Result<String> {
    tdw_core::http_support::read_required_key(API_KEY_ENV, "velodata")
}

// ---------------------------------------------------------------------------
// Funding-rates fetcher
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production Velodata funding-rates HTTP fetcher.
    ///
    /// Calls `GET /funding/rates` with exchange, symbol, `interval=8h`, and
    /// limit query parameters. Requires `TDW_VELODATA_API_KEY` at runtime.
    pub VelodataHttpFundingFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<VelodataFundingQuery, FundingRate> for VelodataHttpFundingFetcher {
    const PROVIDER: &'static str = "velodata";
    const ENDPOINT: &'static str = "funding_rates";

    fn transform_query(params: Value) -> Result<VelodataFundingQuery> {
        funding_query_from_value(&params).map_err(Error::InvalidQuery)
    }

    async fn extract_data(
        &self,
        query: &VelodataFundingQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let api_key = read_api_key()?;
        let url = format!(
            "{}/funding/rates?exchange={}&symbol={}&interval=8h&limit={}",
            self.base_url().trim_end_matches('/'),
            query.exchange,
            query.symbol,
            query.limit,
        );
        get_bytes(&url, &api_key).await
    }

    fn transform_data(
        &self,
        _query: &VelodataFundingQuery,
        raw: Bytes,
    ) -> Result<Vec<FundingRate>> {
        serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("velodata: funding parse: {e}")))
    }
}

// ---------------------------------------------------------------------------
// Liquidations fetcher
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production Velodata aggregated-liquidations HTTP fetcher.
    ///
    /// Calls `GET /liquidations/aggregated` with symbol, `interval=1h`, and
    /// limit query parameters. Requires `TDW_VELODATA_API_KEY` at runtime.
    pub VelodataHttpLiquidationsFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<VelodataLiquidationsQuery, AggregatedLiquidation> for VelodataHttpLiquidationsFetcher {
    const PROVIDER: &'static str = "velodata";
    const ENDPOINT: &'static str = "liquidations_aggregated";

    fn transform_query(params: Value) -> Result<VelodataLiquidationsQuery> {
        liquidations_query_from_value(&params).map_err(Error::InvalidQuery)
    }

    async fn extract_data(
        &self,
        query: &VelodataLiquidationsQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let api_key = read_api_key()?;
        let url = format!(
            "{}/liquidations/aggregated?symbol={}&interval=1h&limit={}",
            self.base_url().trim_end_matches('/'),
            query.symbol,
            query.limit,
        );
        get_bytes(&url, &api_key).await
    }

    fn transform_data(
        &self,
        _query: &VelodataLiquidationsQuery,
        raw: Bytes,
    ) -> Result<Vec<AggregatedLiquidation>> {
        serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("velodata: liquidations parse: {e}")))
    }
}

// ---------------------------------------------------------------------------
// Open-interest fetcher
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production Velodata aggregated open-interest HTTP fetcher.
    ///
    /// Calls `GET /oi/aggregated` with symbol, `interval=1h`, and limit query
    /// parameters. Requires `TDW_VELODATA_API_KEY` at runtime.
    pub VelodataHttpOiFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<VelodataOiQuery, AggregatedOi> for VelodataHttpOiFetcher {
    const PROVIDER: &'static str = "velodata";
    const ENDPOINT: &'static str = "oi_aggregated";

    fn transform_query(params: Value) -> Result<VelodataOiQuery> {
        oi_query_from_value(&params).map_err(Error::InvalidQuery)
    }

    async fn extract_data(&self, query: &VelodataOiQuery, _creds: &Credentials) -> Result<Bytes> {
        let api_key = read_api_key()?;
        let url = format!(
            "{}/oi/aggregated?symbol={}&interval=1h&limit={}",
            self.base_url().trim_end_matches('/'),
            query.symbol,
            query.limit,
        );
        get_bytes(&url, &api_key).await
    }

    fn transform_data(&self, _query: &VelodataOiQuery, raw: Bytes) -> Result<Vec<AggregatedOi>> {
        serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("velodata: oi parse: {e}")))
    }
}

// ---------------------------------------------------------------------------
// Offline transform_data tests (no network needed, always run with --features http)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use serde_json::json;
    use tdw_core::Fetcher;

    use super::{
        VelodataHttpFundingFetcher, VelodataHttpLiquidationsFetcher, VelodataHttpOiFetcher,
    };
    use crate::{
        VelodataFundingQuery, VelodataLiquidationsQuery, VelodataOiQuery, mock_funding_rows,
        mock_liquidations_rows, mock_oi_rows,
    };

    // --- registry entries ---

    #[test]
    fn funding_fetcher_exposes_registry_entry() {
        let entry = VelodataHttpFundingFetcher::registry_entry();

        assert_eq!(entry.provider, "velodata");
        assert_eq!(entry.endpoint, "funding_rates");
    }

    #[test]
    fn liquidations_fetcher_exposes_registry_entry() {
        let entry = VelodataHttpLiquidationsFetcher::registry_entry();

        assert_eq!(entry.provider, "velodata");
        assert_eq!(entry.endpoint, "liquidations_aggregated");
    }

    #[test]
    fn oi_fetcher_exposes_registry_entry() {
        let entry = VelodataHttpOiFetcher::registry_entry();

        assert_eq!(entry.provider, "velodata");
        assert_eq!(entry.endpoint, "oi_aggregated");
    }

    // --- transform_query ---

    #[test]
    fn funding_transform_query_normalizes_inputs() {
        let query = VelodataHttpFundingFetcher::transform_query(json!({
            "exchange": "Binance",
            "symbol": "btcusdt",
            "limit": 50
        }))
        .unwrap_or_else(|e| panic!("transform_query should succeed: {e}"));

        assert_eq!(query.exchange, "binance");
        assert_eq!(query.symbol, "BTCUSDT");
        assert_eq!(query.limit, 50);
    }

    #[test]
    fn funding_transform_query_rejects_injection() {
        assert!(
            VelodataHttpFundingFetcher::transform_query(json!({
                "exchange": "bin/ance",
                "symbol": "BTCUSDT",
                "limit": 10
            }))
            .is_err()
        );
        assert!(
            VelodataHttpFundingFetcher::transform_query(json!({
                "symbol": "BTCUSDT",
                "limit": 10
            }))
            .is_err()
        );
    }

    #[test]
    fn liquidations_transform_query_normalizes_symbol() {
        let query = VelodataHttpLiquidationsFetcher::transform_query(json!({
            "symbol": "btc",
            "limit": 12
        }))
        .unwrap_or_else(|e| panic!("transform_query should succeed: {e}"));

        assert_eq!(query.symbol, "BTC");
        assert_eq!(query.limit, 12);
    }

    #[test]
    fn oi_transform_query_normalizes_symbol() {
        let query = VelodataHttpOiFetcher::transform_query(json!({
            "symbol": "eth",
            "limit": 6
        }))
        .unwrap_or_else(|e| panic!("transform_query should succeed: {e}"));

        assert_eq!(query.symbol, "ETH");
        assert_eq!(query.limit, 6);
    }

    // --- cassette replay: transform_data ---

    fn funding_cassette() -> Bytes {
        let rows = mock_funding_rows("binance", "BTCUSDT");
        Bytes::from(
            serde_json::to_vec(&rows).unwrap_or_else(|e| panic!("mock rows should serialize: {e}")),
        )
    }

    fn liquidations_cassette() -> Bytes {
        let rows = mock_liquidations_rows("BTC");
        Bytes::from(
            serde_json::to_vec(&rows).unwrap_or_else(|e| panic!("mock rows should serialize: {e}")),
        )
    }

    fn oi_cassette() -> Bytes {
        let rows = mock_oi_rows("BTC");
        Bytes::from(
            serde_json::to_vec(&rows).unwrap_or_else(|e| panic!("mock rows should serialize: {e}")),
        )
    }

    #[test]
    fn cassette_replay_decodes_funding_rates() {
        let fetcher = VelodataHttpFundingFetcher::default();
        let query = VelodataFundingQuery::new("binance", "BTCUSDT", 2)
            .unwrap_or_else(|e| panic!("query should build: {e}"));
        let rows = fetcher
            .transform_data(&query, funding_cassette())
            .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].exchange, "binance");
        assert_eq!(rows[0].symbol, "BTCUSDT");
        assert_eq!(rows[0].funding_rate, 0.0001);
        assert_eq!(rows[0].funding_rate_annualized, 0.1095);
        assert_eq!(rows[0].timestamp, 1_704_153_600_000);
    }

    #[test]
    fn cassette_replay_decodes_liquidations() {
        let fetcher = VelodataHttpLiquidationsFetcher::default();
        let query = VelodataLiquidationsQuery::new("BTC", 2)
            .unwrap_or_else(|e| panic!("query should build: {e}"));
        let rows = fetcher
            .transform_data(&query, liquidations_cassette())
            .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].symbol, "BTC");
        assert_eq!(rows[0].long_liquidations, 1_250_000.0);
        assert_eq!(rows[0].short_liquidations, 850_000.0);
        assert_eq!(rows[0].total_liquidations, 2_100_000.0);
    }

    #[test]
    fn cassette_replay_decodes_oi() {
        let fetcher = VelodataHttpOiFetcher::default();
        let query =
            VelodataOiQuery::new("BTC", 2).unwrap_or_else(|e| panic!("query should build: {e}"));
        let rows = fetcher
            .transform_data(&query, oi_cassette())
            .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].symbol, "BTC");
        assert_eq!(rows[0].open_interest, 15_200_000_000.0);
        assert_eq!(rows[0].open_interest_change, 250_000_000.0);
    }

    #[test]
    fn transform_data_rejects_malformed_json() {
        let fetcher = VelodataHttpFundingFetcher::default();
        let query = VelodataFundingQuery::new("binance", "BTCUSDT", 1)
            .unwrap_or_else(|e| panic!("query should build: {e}"));

        assert!(
            fetcher
                .transform_data(&query, Bytes::from_static(b"not json"))
                .is_err()
        );
        assert!(
            fetcher
                .transform_data(&query, Bytes::from_static(b"{}"))
                .is_err()
        );
    }
}
