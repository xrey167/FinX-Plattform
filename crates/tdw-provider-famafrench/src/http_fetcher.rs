//! Real Ken French Data Library HTTP fetcher for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks directly to the public Data Library ftp
//! tree (`https://mba.tuck.dartmouth.edu/.../ftp`) without authentication. Live
//! calls are gated by `TDW_FAMAFRENCH_LIVE=1` in the integration tests.
//!
//! The library ships each research-factor dataset as a ZIP archive of a single
//! `.CSV` member. The fetcher downloads the ZIP (`extract_data`), then in
//! `transform_data` unzips the member in-memory with the pure-Rust `zip` crate
//! (deflate via `flate2`/`miniz_oxide` — no C/zlib binding) and hands the CSV
//! text to the offline [`crate::parse_factor_table`] parser, which skips the
//! descriptive header preamble and normalizes the percent-valued factor table to
//! [`tdw_domain::FactorReturn`] rows.

#![cfg(feature = "http")]

use std::io::{Cursor, Read};

use tdw_core::http_support::prelude::*;
use tdw_domain::{Breakpoint, FactorReturn, PortfolioReturn};

use crate::portfolio::{self, PortfolioKind};
use crate::{BASE_URL, FamaFrenchPortfolioQuery, FamaFrenchQuery, parse_factor_table};

const USER_AGENT: &str = "tdw-provider-famafrench/0.1 (contact@finx.example)";

/// Read the single `.CSV` member named `csv_member` out of a ZIP archive held
/// entirely in memory, returning its UTF-8 (lossy) text. Uses the pure-Rust
/// `zip` crate; no temporary files and no C/zlib dependency.
fn unzip_member(raw: &[u8], csv_member: &str, ctx: &str) -> Result<String> {
    let cursor = Cursor::new(raw);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|error| Error::Provider(format!("{ctx} open zip: {error}")))?;
    // Prefer an exact-name match; fall back to the first `.CSV` member, since the
    // Data Library occasionally prefixes the member with a directory segment.
    let index = (0..archive.len())
        .find(|&i| {
            archive
                .by_index(i)
                .ok()
                .is_some_and(|f| f.name().eq_ignore_ascii_case(csv_member))
        })
        .or_else(|| {
            (0..archive.len()).find(|&i| {
                archive.by_index(i).ok().is_some_and(|f| {
                    std::path::Path::new(f.name())
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("csv"))
                })
            })
        })
        .ok_or_else(|| {
            Error::Provider(format!("{ctx} zip has no CSV member (wanted {csv_member})"))
        })?;
    let mut member = archive
        .by_index(index)
        .map_err(|error| Error::Provider(format!("{ctx} read zip member: {error}")))?;
    let mut buffer = Vec::new();
    member
        .read_to_end(&mut buffer)
        .map_err(|error| Error::Provider(format!("{ctx} extract zip member: {error}")))?;
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

tdw_core::provider_fetcher_struct!(
    /// Production Ken French Data Library research-factor fetcher.
    ///
    /// Standardizes `economy/factors/famafrench` to [`tdw_domain::FactorReturn`]
    /// rows by downloading the selected dataset's ZIP archive and parsing the
    /// inner CSV factor table in-memory.
    pub FamaFrenchHttpFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<FamaFrenchQuery, FactorReturn> for FamaFrenchHttpFetcher {
    const PROVIDER: &'static str = "famafrench";
    const ENDPOINT: &'static str = "economy_factors_famafrench";

    fn transform_query(params: Value) -> Result<FamaFrenchQuery> {
        FamaFrenchQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &FamaFrenchQuery, _creds: &Credentials) -> Result<Bytes> {
        let endpoint = format!(
            "{}/{}",
            self.base_url().trim_end_matches('/'),
            query.dataset().zip_file
        );
        let client =
            tdw_core::http_support::build_client(USER_AGENT, "famafrench http client build")?;
        let response = client
            .get(&endpoint)
            .send()
            .await
            .map_err(|error| Error::Provider(format!("famafrench fetch zip: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "famafrench fetch zip returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|error| Error::Provider(format!("famafrench read zip body: {error}")))
    }

    fn transform_data(&self, query: &FamaFrenchQuery, raw: Bytes) -> Result<Vec<FactorReturn>> {
        let csv = unzip_member(&raw, query.dataset().csv_member, "famafrench")?;
        parse_factor_table(&csv)
            .map_err(|error| Error::Provider(format!("famafrench parse: {error}")))
    }
}

/// Download a portfolio / breakpoint ZIP archive for `query`, returning its raw
/// bytes. Shared by the portfolio and breakpoint fetchers, which differ only in
/// how they normalize the inner wide CSV table.
async fn fetch_portfolio_zip(base_url: &str, query: &FamaFrenchPortfolioQuery) -> Result<Bytes> {
    let endpoint = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        query.dataset().zip_file
    );
    let client = tdw_core::http_support::build_client(USER_AGENT, "famafrench http client build")?;
    let response = client
        .get(&endpoint)
        .send()
        .await
        .map_err(|error| Error::Provider(format!("famafrench fetch zip: {error}")))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Provider(format!(
            "famafrench fetch zip returned {status}: {body}"
        )));
    }
    response
        .bytes()
        .await
        .map_err(|error| Error::Provider(format!("famafrench read zip body: {error}")))
}

tdw_core::provider_fetcher_struct!(
    /// Production Ken French portfolio-formation returns fetcher (OpenBB-parity
    /// **P4W9**).
    ///
    /// Standardizes the `economy/factors/famafrench/{us,regional,country}_portfolio_returns`
    /// and `economy/factors/famafrench/international_index_returns` routes to
    /// long-format [`tdw_domain::PortfolioReturn`] rows by downloading the
    /// resolved dataset's ZIP archive and parsing the inner wide CSV table.
    pub FamaFrenchPortfolioHttpFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<FamaFrenchPortfolioQuery, PortfolioReturn> for FamaFrenchPortfolioHttpFetcher {
    const PROVIDER: &'static str = "famafrench";
    const ENDPOINT: &'static str = "economy_factors_famafrench_portfolio";

    fn transform_query(params: Value) -> Result<FamaFrenchPortfolioQuery> {
        FamaFrenchPortfolioQuery::from_value(&params)
    }

    async fn extract_data(
        &self,
        query: &FamaFrenchPortfolioQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        fetch_portfolio_zip(self.base_url(), query).await
    }

    fn transform_data(
        &self,
        query: &FamaFrenchPortfolioQuery,
        raw: Bytes,
    ) -> Result<Vec<PortfolioReturn>> {
        let dataset = query.dataset();
        let csv = unzip_member(&raw, dataset.csv_member, "famafrench portfolio")?;
        Ok(portfolio::parse_wide_table(&csv, PortfolioKind::Return)
            .into_iter()
            .map(|cell| PortfolioReturn {
                date: cell.date,
                portfolio: cell.label,
                value: cell.value,
            })
            .collect())
    }
}

tdw_core::provider_fetcher_struct!(
    /// Production Ken French portfolio-formation breakpoints fetcher
    /// (OpenBB-parity **P4W9**).
    ///
    /// Standardizes `economy/factors/famafrench/breakpoints` to long-format
    /// [`tdw_domain::Breakpoint`] rows. Breakpoint levels carry through unscaled
    /// (the source publishes them as levels, not percent).
    pub FamaFrenchBreakpointsHttpFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<FamaFrenchPortfolioQuery, Breakpoint> for FamaFrenchBreakpointsHttpFetcher {
    const PROVIDER: &'static str = "famafrench";
    const ENDPOINT: &'static str = "economy_factors_famafrench_breakpoints";

    fn transform_query(params: Value) -> Result<FamaFrenchPortfolioQuery> {
        FamaFrenchPortfolioQuery::from_value(&params)
    }

    async fn extract_data(
        &self,
        query: &FamaFrenchPortfolioQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        fetch_portfolio_zip(self.base_url(), query).await
    }

    fn transform_data(
        &self,
        query: &FamaFrenchPortfolioQuery,
        raw: Bytes,
    ) -> Result<Vec<Breakpoint>> {
        let dataset = query.dataset();
        let csv = unzip_member(&raw, dataset.csv_member, "famafrench breakpoints")?;
        Ok(portfolio::parse_wide_table(&csv, PortfolioKind::Breakpoint)
            .into_iter()
            .map(|cell| Breakpoint {
                date: cell.date,
                breakpoint: cell.label,
                value: cell.value,
            })
            .collect())
    }
}
