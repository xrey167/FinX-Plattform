//! Offline Parquet manifest round-trip: build a dataset manifest, read its
//! totals, and verify its checksums. No network, no docker.
//!
//! Run with: `cargo run -p tdw-storage-parquet --example tdw-storage-parquet-basic`

use tdw_storage_parquet::{ParquetDatasetManifest, ParquetFile};

fn main() -> tdw_storage_parquet::Result<()> {
    let files = vec![
        ParquetFile::new("s3://bucket/raw/ohlcv-001.parquet", 42, 4096)?,
        ParquetFile::new("s3://bucket/raw/ohlcv-002.parquet", 58, 5120)?,
    ];
    let manifest = ParquetDatasetManifest::new("raw.market_data_bar", files)?;

    // Integrity check: recomputes per-file and whole-manifest FNV checksums.
    manifest.verify_checksums()?;

    assert_eq!(manifest.total_rows(), 100);
    assert_eq!(manifest.total_bytes(), 9216);
    println!(
        "manifest ok: table={}, files={}, total_rows={}, total_bytes={}",
        manifest.table,
        manifest.files.len(),
        manifest.total_rows(),
        manifest.total_bytes()
    );
    Ok(())
}
