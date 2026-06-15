//! Real `BizToc` HTTP fetcher for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to the `BizToc` `RapidAPI` endpoint
//! (`https://biztoc.p.rapidapi.com`). Requires a free `RapidAPI` key (read from
//! `BIZTOC_API_KEY`) sent via the `X-RapidAPI-Key` header alongside the
//! `X-RapidAPI-Host` header. Live calls are gated by `TDW_BIZTOC_LIVE=1` in the
//! integration tests.
//!
//! The `/news/latest` endpoint returns a JSON array of news items. Each item is
//! normalized to the standard [`tdw_domain::NewsArticle`] model so no raw
//! `BizToc` shape leaks out. Fields beyond `id`/`title` are tolerated as absent
//! (serde defaults) because the free `RapidAPI` tier omits some of them.

#![cfg(feature = "http")]

use serde::Deserialize;
use tdw_core::http_support::prelude::*;
use tdw_domain::NewsArticle;

use crate::{API_KEY_ENV, BASE_URL, BiztocWorldNewsQuery, RAPIDAPI_HOST};

const USER_AGENT: &str = "tdw-provider-biztoc/0.1";

/// One `BizToc` news item from the `/news/latest` array.
///
/// Field names follow the documented `BizToc` `RapidAPI` response shape. Every
/// field except `title` is defaulted so a partial item still decodes.
#[derive(Deserialize)]
struct WireNewsItem {
    #[serde(default)]
    id: String,
    title: String,
    #[serde(default)]
    url: String,
    /// Publication timestamp (RFC 3339), e.g. `"2024-01-25T09:30:00Z"`.
    #[serde(default)]
    created: String,
    /// Short summary / excerpt of the article body.
    #[serde(default)]
    excerpt: Option<String>,
    /// Originating domain, e.g. `"reuters.com"`.
    #[serde(default)]
    domain: Option<String>,
    /// Tags / topics the article is filed under.
    #[serde(default)]
    tags: Vec<String>,
}

/// Read the required `RapidAPI` key, erroring when `BIZTOC_API_KEY` is unset,
/// through the shared `read_required_key` helper.
fn read_api_key() -> Result<String> {
    tdw_core::http_support::read_required_key(API_KEY_ENV, "biztoc")
}

/// Normalize a raw `BizToc` wire item into the standard [`NewsArticle`].
///
/// Field mapping: `id`→`id` (URL fallback when blank), `title`→`title`,
/// `created`→`published_at`, `excerpt`→`text`, `url`→`url`, `domain`→`source`,
/// `tags`→`symbols`. `author` is always `None` (`BizToc` is an aggregator with
/// no byline).
fn map_news_article(wire: WireNewsItem) -> NewsArticle {
    let id = if wire.id.trim().is_empty() {
        wire.url.clone()
    } else {
        wire.id
    };
    NewsArticle {
        id,
        title: wire.title,
        published_at: wire.created,
        text: wire.excerpt,
        url: (!wire.url.trim().is_empty()).then_some(wire.url),
        source: wire.domain,
        author: None,
        symbols: wire.tags,
    }
}

tdw_core::provider_fetcher_struct!(
    /// Production `BizToc` world-news fetcher.
    ///
    /// Standardizes the `news/world` route's `BizToc` leg to
    /// [`tdw_domain::NewsArticle`] rows from the `BizToc` `RapidAPI`
    /// `/news/latest` endpoint.
    pub BiztocWorldNewsFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<BiztocWorldNewsQuery, NewsArticle> for BiztocWorldNewsFetcher {
    const PROVIDER: &'static str = crate::PROVIDER_ID;
    const ENDPOINT: &'static str = "news_world";

    fn transform_query(params: Value) -> Result<BiztocWorldNewsQuery> {
        // The `news/world` catalog route uses `StandardParams`, which exposes the
        // optional `limit` (not `page_size`). Accept the provider-native
        // `page_size` first, fall back to the standard `limit`, and default to 20
        // so a request without either still resolves instead of erroring.
        let page_size = params
            .get("page_size")
            .or_else(|| params.get("limit"))
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(20);
        BiztocWorldNewsQuery::new(page_size).map_err(|error| Error::InvalidQuery(error.to_string()))
    }

    async fn extract_data(
        &self,
        _query: &BiztocWorldNewsQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let api_key = read_api_key()?;
        let client = tdw_core::http_support::build_client(USER_AGENT, "biztoc client build")?;
        let endpoint = format!("{}/news/latest", self.base_url().trim_end_matches('/'));
        let response = client
            .get(&endpoint)
            .header("X-RapidAPI-Key", api_key)
            .header("X-RapidAPI-Host", RAPIDAPI_HOST)
            .send()
            .await
            .map_err(|error| Error::Provider(format!("biztoc news request: {error}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "biztoc news returned {status}: {body}"
            )));
        }

        response
            .bytes()
            .await
            .map_err(|error| Error::Provider(format!("biztoc news read body: {error}")))
    }

    fn transform_data(&self, query: &BiztocWorldNewsQuery, raw: Bytes) -> Result<Vec<NewsArticle>> {
        let wire: Vec<WireNewsItem> = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("biztoc news parse: {error}")))?;
        Ok(wire
            .into_iter()
            .take(query.page_size as usize)
            .map(map_news_article)
            .collect())
    }
}
