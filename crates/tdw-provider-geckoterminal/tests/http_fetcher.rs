//! Tests for the real `GeckoTerminal` HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded `GeckoTerminal`
//! response shapes. The live test is additionally gated by
//! `TDW_GECKOTERMINAL_LIVE=1` and requires a network connection (no API key
//! needed).

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::{Credentials, Fetcher};
use tdw_provider_geckoterminal::http_fetcher::{fetch_trending_raw, parse_pool_list};
use tdw_provider_geckoterminal::{GeckoTerminalHttpFetcher, GeckoTerminalPoolQuery};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Pool address used across all tests — the USDC/WETH 0.05% Uniswap V3 pool.
const ETH_POOL: &str = "0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640";

fn sample_query() -> GeckoTerminalPoolQuery {
    GeckoTerminalHttpFetcher::transform_query(json!({
        "network": "eth",
        "pool_address": ETH_POOL,
    }))
    .unwrap_or_else(|e| panic!("query should transform: {e}"))
}

fn pool_cassette() -> Bytes {
    Bytes::from(
        json!({
            "data": {
                "id": "eth_0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640",
                "type": "pool",
                "attributes": {
                    "name": "USDC/WETH 0.05%",
                    "base_token_price_usd": "1.0001",
                    "quote_token_price_usd": "3125.50",
                    "base_token_price_quote_token": "0.00032",
                    "quote_token_price_base_token": "3125.50",
                    "pool_created_at": "2021-05-04T00:00:00Z",
                    "reserve_in_usd": "125000000.5",
                    "fdv_usd": "0",
                    "volume_usd": {
                        "h24": "85000000.0",
                        "h6": "22000000.0",
                        "h1": "4500000.0"
                    },
                    "price_change_percentage": {
                        "h24": "-0.25",
                        "h6": "0.10",
                        "h1": "0.05"
                    }
                }
            }
        })
        .to_string()
        .into_bytes(),
    )
}

fn trending_cassette() -> Bytes {
    Bytes::from(
        json!({
            "data": [
                {
                    "id": "eth_0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640",
                    "type": "pool",
                    "attributes": {
                        "name": "TOKEN/WETH",
                        "volume_usd": {
                            "h24": "12000000"
                        }
                    }
                },
                {
                    "id": "eth_0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
                    "type": "pool",
                    "attributes": {
                        "name": "USDC/ETH",
                        "volume_usd": {
                            "h24": "5000000"
                        }
                    }
                }
            ]
        })
        .to_string()
        .into_bytes(),
    )
}

// ---------------------------------------------------------------------------
// transform_query tests
// ---------------------------------------------------------------------------

#[test]
fn transform_query_normalises_network_and_accepts_evm_address() {
    let q = GeckoTerminalHttpFetcher::transform_query(json!({
        "network": "ETH",
        "pool_address": ETH_POOL,
    }))
    .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    assert_eq!(q.network, "eth");
    assert_eq!(q.pool_address, ETH_POOL);
}

#[test]
fn transform_query_rejects_missing_network() {
    let err = GeckoTerminalHttpFetcher::transform_query(json!({
        "pool_address": ETH_POOL,
    }))
    .expect_err("missing network must be rejected");

    assert!(err.to_string().contains("network"), "err={err}");
}

#[test]
fn transform_query_rejects_path_injection_in_network() {
    let err = GeckoTerminalHttpFetcher::transform_query(json!({
        "network": "eth/../../secret",
        "pool_address": ETH_POOL,
    }))
    .expect_err("path injection must be rejected");

    assert!(err.to_string().contains("network"), "err={err}");
}

#[test]
fn transform_query_rejects_short_pool_address() {
    let err = GeckoTerminalHttpFetcher::transform_query(json!({
        "network": "eth",
        "pool_address": "0xDEAD",
    }))
    .expect_err("short address must be rejected");

    assert!(err.to_string().contains("pool address"), "err={err}");
}

// ---------------------------------------------------------------------------
// transform_data (cassette) tests
// ---------------------------------------------------------------------------

#[test]
fn cassette_replay_decodes_pool_into_dex_pool() {
    let fetcher = GeckoTerminalHttpFetcher::default();
    let query = sample_query();
    let rows = fetcher
        .transform_data(&query, pool_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    let pool = &rows[0];
    assert_eq!(pool.network, "eth");
    assert_eq!(pool.pool_address, ETH_POOL);
    assert_eq!(pool.name, "USDC/WETH 0.05%");
    assert_eq!(pool.id, "eth_0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640");
    assert_eq!(pool.base_token_price_usd, Some(1.0001));
    assert_eq!(pool.quote_token_price_usd, Some(3125.50));
    assert_eq!(pool.reserve_in_usd, Some(125_000_000.5));
    assert_eq!(pool.volume_usd_h24, Some(85_000_000.0));
    assert_eq!(pool.volume_usd_h6, Some(22_000_000.0));
    assert_eq!(pool.volume_usd_h1, Some(4_500_000.0));
    assert_eq!(pool.price_change_h24, Some(-0.25));
    assert_eq!(pool.price_change_h6, Some(0.10));
    assert_eq!(pool.price_change_h1, Some(0.05));
    assert_eq!(
        pool.pool_created_at.as_deref(),
        Some("2021-05-04T00:00:00Z")
    );
}

#[test]
fn cassette_replay_handles_missing_optional_fields() {
    let fetcher = GeckoTerminalHttpFetcher::default();
    let query = sample_query();
    let minimal = Bytes::from(
        json!({
            "data": {
                "id": "eth_0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640",
                "type": "pool",
                "attributes": {
                    "name": "MINIMAL/POOL"
                }
            }
        })
        .to_string()
        .into_bytes(),
    );
    let rows = fetcher
        .transform_data(&query, minimal)
        .unwrap_or_else(|e| panic!("transform_data must handle minimal payload: {e}"));

    assert_eq!(rows.len(), 1);
    let pool = &rows[0];
    assert_eq!(pool.name, "MINIMAL/POOL");
    assert!(pool.base_token_price_usd.is_none());
    assert!(pool.volume_usd_h24.is_none());
    assert!(pool.price_change_h24.is_none());
    assert!(pool.pool_created_at.is_none());
}

#[test]
fn cassette_replay_surfaces_json_parse_error() {
    let fetcher = GeckoTerminalHttpFetcher::default();
    let query = sample_query();
    let bad = Bytes::from(b"not json".as_slice());
    let err = fetcher
        .transform_data(&query, bad)
        .expect_err("invalid JSON must return an error");

    assert!(
        err.to_string().contains("geckoterminal parse_json"),
        "err={err}"
    );
}

// ---------------------------------------------------------------------------
// parse_pool_list (cassette) tests
// ---------------------------------------------------------------------------

#[test]
fn cassette_parse_pool_list_decodes_trending_response() {
    let pools = parse_pool_list(trending_cassette(), "eth")
        .unwrap_or_else(|e| panic!("parse_pool_list must succeed: {e}"));

    assert_eq!(pools.len(), 2, "pools={pools:#?}");
    assert_eq!(pools[0].network, "eth");
    assert_eq!(pools[0].name, "TOKEN/WETH");
    assert_eq!(pools[0].volume_usd_h24, Some(12_000_000.0));
    assert_eq!(pools[1].name, "USDC/ETH");
}

#[test]
fn cassette_parse_pool_list_returns_empty_for_empty_data() {
    let empty = Bytes::from(json!({"data": []}).to_string().into_bytes());
    let pools =
        parse_pool_list(empty, "eth").unwrap_or_else(|e| panic!("empty data must succeed: {e}"));

    assert!(pools.is_empty());
}

// ---------------------------------------------------------------------------
// Live integration test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_geckoterminal_returns_pool_when_env_var_set() {
    if std::env::var("TDW_GECKOTERMINAL_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_GECKOTERMINAL_LIVE != 1; skipping live GeckoTerminal integration test");
        return;
    }

    let fetcher = GeckoTerminalHttpFetcher::default();
    let query = sample_query();
    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|e| panic!("live extract_data must succeed: {e}"));
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("live transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 1, "expected exactly one pool");
    assert_eq!(rows[0].network, "eth");
    assert!(
        rows[0].volume_usd_h24.is_some(),
        "live response must have h24 volume"
    );
}

#[tokio::test]
async fn live_geckoterminal_trending_returns_pools_when_env_var_set() {
    if std::env::var("TDW_GECKOTERMINAL_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_GECKOTERMINAL_LIVE != 1; skipping live trending test");
        return;
    }

    use tdw_provider_geckoterminal::BASE_URL;

    let raw = fetch_trending_raw(BASE_URL, "eth")
        .await
        .unwrap_or_else(|e| panic!("live trending must succeed: {e}"));
    let pools = parse_pool_list(raw, "eth")
        .unwrap_or_else(|e| panic!("live parse_pool_list must succeed: {e}"));

    assert!(
        !pools.is_empty(),
        "live trending must return at least one pool"
    );
}
