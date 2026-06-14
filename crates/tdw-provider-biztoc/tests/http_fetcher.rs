//! Tests for the real `BizToc` HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse the documented `BizToc`
//! `RapidAPI` response shape without network access. The live test is
//! additionally gated by `TDW_BIZTOC_LIVE=1` and requires a free `BIZTOC_API_KEY`.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_biztoc::BiztocWorldNewsFetcher;
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

#[test]
fn base_url_uses_tls() {
    assert!(
        tdw_provider_biztoc::BASE_URL.starts_with("https://"),
        "biztoc base URL must use TLS"
    );
}

fn news_cassette() -> Bytes {
    cassette_bytes!([
        {
            "id": "biztoc-1",
            "title": "Markets rally on rate-cut hopes",
            "url": "https://biztoc.com/x/markets-rally",
            "created": "2024-01-25T09:30:00Z",
            "excerpt": "Global equities climbed as investors priced in cuts.",
            "domain": "reuters.com",
            "tags": ["markets", "rates"]
        },
        {
            "id": "",
            "title": "Oil steadies after volatile session",
            "url": "https://biztoc.com/x/oil-steadies",
            "created": "2024-01-25T08:00:00Z"
        }
    ])
}

#[test]
fn news_cassette_normalises_to_news_article() {
    let fetcher = BiztocWorldNewsFetcher::default();
    let query = BiztocWorldNewsFetcher::transform_query(json!({ "page_size": 10 }))
        .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let rows = fetcher
        .transform_data(&query, news_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 2);
    // First item: all fields mapped.
    assert_eq!(rows[0].id, "biztoc-1");
    assert_eq!(rows[0].title, "Markets rally on rate-cut hopes");
    assert_eq!(rows[0].published_at, "2024-01-25T09:30:00Z");
    assert_eq!(
        rows[0].text.as_deref(),
        Some("Global equities climbed as investors priced in cuts.")
    );
    assert_eq!(
        rows[0].url.as_deref(),
        Some("https://biztoc.com/x/markets-rally")
    );
    assert_eq!(rows[0].source.as_deref(), Some("reuters.com"));
    assert_eq!(rows[0].author, None);
    assert_eq!(
        rows[0].symbols,
        vec!["markets".to_string(), "rates".to_string()]
    );
    // Second item: blank id falls back to the URL.
    assert_eq!(rows[1].id, "https://biztoc.com/x/oil-steadies");
}

#[test]
fn news_transform_respects_page_size_cap() {
    let fetcher = BiztocWorldNewsFetcher::default();
    let query = BiztocWorldNewsFetcher::transform_query(json!({ "page_size": 1 }))
        .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = fetcher
        .transform_data(&query, news_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));
    assert_eq!(rows.len(), 1, "page_size caps the returned rows");
}

#[test]
fn news_transform_rejects_malformed_json() {
    let fetcher = BiztocWorldNewsFetcher::default();
    let query = BiztocWorldNewsFetcher::transform_query(json!({ "page_size": 10 }))
        .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let err = fetcher
        .transform_data(&query, Bytes::from_static(b"{not json"))
        .expect_err("malformed JSON must error");
    assert!(err.to_string().contains("biztoc news parse"), "got: {err}");
}

#[test]
fn transform_query_rejects_invalid_page_size() {
    assert!(BiztocWorldNewsFetcher::transform_query(json!({})).is_err());
    assert!(BiztocWorldNewsFetcher::transform_query(json!({ "page_size": 0 })).is_err());
    assert!(BiztocWorldNewsFetcher::transform_query(json!({ "page_size": 101 })).is_err());
}

#[tokio::test]
async fn live_biztoc_world_news_returns_rows_when_env_set() {
    if std::env::var("TDW_BIZTOC_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_BIZTOC_LIVE != 1; skipping live biztoc world-news test");
        return;
    }
    let fetcher = BiztocWorldNewsFetcher::default();
    let query = BiztocWorldNewsFetcher::transform_query(json!({ "page_size": 5 }))
        .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live biztoc world news must include rows");
}
