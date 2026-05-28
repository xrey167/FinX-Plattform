//! End-to-end functional smoke for G009.
//!
//! Composes the bootstrap pieces along the path the production service
//! request will travel:
//!
//! ```text
//! caller
//!   -> tdw_service_api::fetch_equity_historical
//!        -> tdw_runtime::CommandRunner::run
//!             -> tdw_provider_fileset::FilesetEquityHistoricalFetcher
//!                  -> deterministic fixture rows
//! -> serde_json (OBBject -> bytes)
//! -> tdw_storage_fs::LocalBlobEngine put + get (roundtrip)
//! -> SmokeReport
//! ```
//!
//! The smoke is **deterministic** (fixture data, per-process scratch dir)
//! and **offline** (no network, no external services), so it is safe to
//! run as the baseline "still works" check in CI and from both the
//! `tdw-service` and `tdw-cli` binaries. Future production tranches
//! (G010+) extend the participant set; this smoke is the floor every
//! subsequent change has to keep green.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use serde::Serialize;
use tdw_core::{BlobEngine, Error, Result};
use tdw_storage_fs::LocalBlobEngine;

/// Structured report emitted by [`run_end_to_end_smoke`]. Serialized to JSON
/// by the binaries; asserted on field-by-field by the integration tests.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SmokeReport {
    pub provider: String,
    pub endpoint: String,
    pub query_symbol: String,
    pub rows_fetched: usize,
    pub blob_key: String,
    pub blob_bytes_written: usize,
    pub blob_bytes_read: usize,
    pub roundtrip_ok: bool,
    pub storage_root: String,
}

/// Run the G009 end-to-end smoke against the supplied storage root.
///
/// 1. Drive a fetch through [`tdw_service_api::fetch_equity_historical`]
///    (which internally composes [`tdw_runtime::CommandRunner`] with
///    [`tdw_provider_fileset::FilesetEquityHistoricalFetcher`]).
/// 2. Serialize the resulting [`tdw_core::OBBject`] to JSON bytes.
/// 3. Write the bytes to a [`LocalBlobEngine`] rooted at `storage_root`.
/// 4. Read them back and confirm a byte-exact roundtrip.
///
/// Returns a populated [`SmokeReport`] on success. Any failure (provider
/// error, serialization, storage I/O, roundtrip mismatch) yields a
/// [`tdw_core::Error`].
///
/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub async fn run_end_to_end_smoke(symbol: &str, storage_root: PathBuf) -> Result<SmokeReport> {
    let object = tdw_service_api::fetch_equity_historical("fileset", symbol)?;

    let bytes = serde_json::to_vec(&object)
        .map(Bytes::from)
        .map_err(|error| Error::Provider(error.to_string()))?;

    let engine = LocalBlobEngine::new(&storage_root);
    let blob_key = format!("smoke/{}.json", object.rows[0].symbol);

    engine
        .put_object(&blob_key, bytes.clone(), "application/json")
        .await?;
    let roundtrip = engine.get_object(&blob_key).await?;

    Ok(SmokeReport {
        provider: object.provider.clone(),
        endpoint: object.endpoint.clone(),
        query_symbol: object.rows[0].symbol.clone(),
        rows_fetched: object.rows.len(),
        blob_key,
        blob_bytes_written: bytes.len(),
        blob_bytes_read: roundtrip.len(),
        roundtrip_ok: roundtrip == bytes,
        storage_root: storage_root.display().to_string(),
    })
}

/// Allocate a unique per-process scratch directory under the OS temp dir.
/// Suitable for invocations of [`run_end_to_end_smoke`] that want to
/// manage their own storage root without colliding with parallel runs.
pub fn allocate_storage_root(prefix: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    std::env::temp_dir().join(format!("{prefix}-{pid}-{nanos}-{seq}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn smoke_reports_roundtrip_for_fileset_aapl() {
        let root = allocate_storage_root("tdw-smoke-unit");
        let report = run_end_to_end_smoke("AAPL", root.clone())
            .await
            .unwrap_or_else(|error| panic!("smoke must succeed: {error}"));

        assert_eq!(report.provider, "fileset");
        assert_eq!(report.endpoint, "equity_historical");
        assert_eq!(report.query_symbol, "AAPL");
        assert_eq!(report.rows_fetched, 2);
        assert_eq!(report.blob_key, "smoke/AAPL.json");
        assert_eq!(report.blob_bytes_written, report.blob_bytes_read);
        assert!(report.roundtrip_ok);

        let _ = std::fs::remove_dir_all(&root);
    }
}
