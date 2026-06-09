#![cfg(feature = "http")]
//! Real FINRA Market Data HTTP fetchers for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks directly to the public FINRA data API
//! (`https://api.finra.org/data/group`) without authentication.
//! Live calls are additionally gated by `TDW_FINRA_LIVE=1` in the integration
//! test.
//!
//! The FINRA API returns pipe-delimited plain text, not JSON.

use tdw_core::http_support::prelude::*;

use crate::{
    BASE_URL, FinraOtcSummaryQuery, FinraOtcSummaryRecord, FinraShortInterestQuery,
    FinraShortInterestRecord, parse_otc_summary_response, parse_short_interest_response,
};

// ── Short-interest fetcher ────────────────────────────────────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production FINRA consolidated short-interest HTTP fetcher.
    ///
    /// Calls `GET /OTCMarket/block/CONSOLIDATEDSHORTINTERESTdata` with
    /// pipe-delimited CSV response.
    pub FinraShortInterestHttpFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<FinraShortInterestQuery, FinraShortInterestRecord> for FinraShortInterestHttpFetcher {
    const PROVIDER: &'static str = "finra";
    const ENDPOINT: &'static str = "short_interest";

    fn transform_query(params: Value) -> Result<FinraShortInterestQuery> {
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .map(|v| v as u32)
            .unwrap_or(25);
        let offset = params
            .get("offset")
            .and_then(Value::as_u64)
            .map(|v| v as u32)
            .unwrap_or(0);
        FinraShortInterestQuery::new(limit, offset).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &FinraShortInterestQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let url = format!(
            "{}/OTCMarket/block/CONSOLIDATEDSHORTINTERESTdata\
             ?limit={}&offset={}&delimiter=|\
             &fields=issuerName,symbol,marketClassCode,currentShortInterest,\
             previousShortInterest,percentChangeinShortInterest,\
             avgDailyShareVolume,daysToCoverShortInterest,settlementDate",
            self.base_url().trim_end_matches('/'),
            query.limit,
            query.offset,
        );

        let client = build_client()?;
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("finra short_interest extract_data: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_else(|_| String::new());
            return Err(Error::Provider(format!(
                "finra short_interest returned {status}: {body}"
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("finra short_interest read body: {e}")))?;

        Ok(bytes)
    }

    fn transform_data(
        &self,
        _query: &FinraShortInterestQuery,
        raw: Bytes,
    ) -> Result<Vec<FinraShortInterestRecord>> {
        let text = std::str::from_utf8(&raw)
            .map_err(|e| Error::Provider(format!("finra short_interest utf8: {e}")))?;
        parse_short_interest_response(text)
            .map_err(|e| Error::Provider(format!("finra short_interest parse: {e}")))
    }
}

// ── OTC summary fetcher ───────────────────────────────────────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production FINRA weekly OTC summary HTTP fetcher.
    ///
    /// Calls `GET /OTCMarket/block/WEEKLYSUMMARYdata` with pipe-delimited CSV
    /// response.
    pub FinraOtcSummaryHttpFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<FinraOtcSummaryQuery, FinraOtcSummaryRecord> for FinraOtcSummaryHttpFetcher {
    const PROVIDER: &'static str = "finra";
    const ENDPOINT: &'static str = "otc_summary";

    fn transform_query(params: Value) -> Result<FinraOtcSummaryQuery> {
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .map(|v| v as u32)
            .unwrap_or(10);
        FinraOtcSummaryQuery::new(limit).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &FinraOtcSummaryQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let url = format!(
            "{}/OTCMarket/block/WEEKLYSUMMARYdata\
             ?limit={}&fields=tradeReportDate,marketParticipantIdentifier,\
             totalShareQuantity,totalTradeCount",
            self.base_url().trim_end_matches('/'),
            query.limit,
        );

        let client = build_client()?;
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("finra otc_summary extract_data: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_else(|_| String::new());
            return Err(Error::Provider(format!(
                "finra otc_summary returned {status}: {body}"
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("finra otc_summary read body: {e}")))?;

        Ok(bytes)
    }

    fn transform_data(
        &self,
        _query: &FinraOtcSummaryQuery,
        raw: Bytes,
    ) -> Result<Vec<FinraOtcSummaryRecord>> {
        let text = std::str::from_utf8(&raw)
            .map_err(|e| Error::Provider(format!("finra otc_summary utf8: {e}")))?;
        parse_otc_summary_response(text)
            .map_err(|e| Error::Provider(format!("finra otc_summary parse: {e}")))
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

fn build_client() -> Result<Client> {
    Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| Error::Provider(format!("finra http client build: {e}")))
}

// ── Inline unit tests for transform_data / transform_query (no network) ──────

#[cfg(test)]
mod tests {
    use super::*;

    fn short_interest_cassette() -> Bytes {
        Bytes::from(concat!(
            "APPLE INC|AAPL|G|108234568|107982345|0.23|56789012|1.9|2024-01-15\n",
            "TESLA INC|TSLA|G|55000000|54000000|1.85|22000000|2.5|2024-01-15\n",
        ))
    }

    fn otc_summary_cassette() -> Bytes {
        Bytes::from(concat!(
            "2024-01-15|MKTX|1234567|890\n",
            "2024-01-08|GSCO|9876543|1200\n",
        ))
    }

    // ── transform_query ───────────────────────────────────────────────────────

    #[test]
    fn transform_query_short_interest_defaults() {
        let q = FinraShortInterestHttpFetcher::transform_query(serde_json::json!({}))
            .expect("default params must build");
        assert_eq!(q.limit, 25);
        assert_eq!(q.offset, 0);
    }

    #[test]
    fn transform_query_short_interest_accepts_explicit_values() {
        let q = FinraShortInterestHttpFetcher::transform_query(
            serde_json::json!({"limit": 100, "offset": 50}),
        )
        .expect("explicit params must build");
        assert_eq!(q.limit, 100);
        assert_eq!(q.offset, 50);
    }

    #[test]
    fn transform_query_short_interest_rejects_zero_limit() {
        assert!(
            FinraShortInterestHttpFetcher::transform_query(serde_json::json!({"limit": 0}))
                .is_err(),
            "zero limit must be rejected"
        );
    }

    #[test]
    fn transform_query_otc_summary_defaults() {
        let q = FinraOtcSummaryHttpFetcher::transform_query(serde_json::json!({}))
            .expect("default params must build");
        assert_eq!(q.limit, 10);
    }

    #[test]
    fn transform_query_otc_summary_rejects_over_max() {
        assert!(
            FinraOtcSummaryHttpFetcher::transform_query(serde_json::json!({"limit": 9999}))
                .is_err(),
            "over-max limit must be rejected"
        );
    }

    // ── transform_data ────────────────────────────────────────────────────────

    #[test]
    fn transform_data_short_interest_parses_cassette() {
        let fetcher = FinraShortInterestHttpFetcher::default();
        let query = FinraShortInterestQuery::new(25, 0).expect("valid query");
        let rows = fetcher
            .transform_data(&query, short_interest_cassette())
            .expect("transform_data must succeed");
        assert_eq!(rows.len(), 2, "expected 2 rows, got {rows:#?}");
        assert_eq!(rows[0].issuer_name, "APPLE INC");
        assert_eq!(rows[0].symbol, "AAPL");
        assert_eq!(rows[0].current_short_interest, 108_234_568);
        assert_eq!(rows[0].settlement_date, "2024-01-15");
        assert_eq!(rows[1].symbol, "TSLA");
    }

    #[test]
    fn transform_data_otc_summary_parses_cassette() {
        let fetcher = FinraOtcSummaryHttpFetcher::default();
        let query = FinraOtcSummaryQuery::new(10).expect("valid query");
        let rows = fetcher
            .transform_data(&query, otc_summary_cassette())
            .expect("transform_data must succeed");
        assert_eq!(rows.len(), 2, "expected 2 rows, got {rows:#?}");
        assert_eq!(rows[0].trade_report_date, "2024-01-15");
        assert_eq!(rows[0].market_participant_identifier, "MKTX");
        assert_eq!(rows[0].total_share_quantity, 1_234_567);
        assert_eq!(rows[1].total_trade_count, 1200);
    }

    #[test]
    fn transform_data_short_interest_empty_body_returns_empty_vec() {
        let fetcher = FinraShortInterestHttpFetcher::default();
        let query = FinraShortInterestQuery::new(25, 0).expect("valid query");
        let rows = fetcher
            .transform_data(&query, Bytes::new())
            .expect("empty body must not error");
        assert!(rows.is_empty());
    }

    #[test]
    fn transform_data_otc_summary_empty_body_returns_empty_vec() {
        let fetcher = FinraOtcSummaryHttpFetcher::default();
        let query = FinraOtcSummaryQuery::new(10).expect("valid query");
        let rows = fetcher
            .transform_data(&query, Bytes::new())
            .expect("empty body must not error");
        assert!(rows.is_empty());
    }
}
