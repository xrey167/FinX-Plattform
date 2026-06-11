//! Offline Parquet manifest round-trip: build a dataset manifest, read its
//! totals, and verify SHA-256 content checksums. No network, no docker.
//!
//! Run with: `cargo run -p tdw-storage-parquet --example tdw-storage-parquet-basic`

use std::io::Cursor;

use tdw_storage_parquet::{ParquetDatasetManifest, ParquetFile, VerifyOutcome};

fn main() -> tdw_storage_parquet::Result<()> {
    let content_a = b"parquet-bytes-file-001";
    let content_b = b"parquet-bytes-file-002";

    let files = vec![
        ParquetFile::from_reader(
            "s3://bucket/raw/ohlcv-001.parquet",
            42,
            content_a.len() as u64,
            Cursor::new(content_a),
        )?,
        ParquetFile::from_reader(
            "s3://bucket/raw/ohlcv-002.parquet",
            58,
            content_b.len() as u64,
            Cursor::new(content_b),
        )?,
    ];
    let manifest = ParquetDatasetManifest::new("raw.market_data_bar", files)?;

    // Integrity check: re-reads each file and verifies SHA-256 content digest.
    let mut call_count = 0usize;
    let outcomes = manifest.verify_checksums(|path| {
        call_count += 1;
        if path.ends_with("001.parquet") {
            Ok(Cursor::new(content_a.as_ref()))
        } else {
            Ok(Cursor::new(content_b.as_ref()))
        }
    })?;
    assert!(outcomes.iter().all(|o| *o == VerifyOutcome::Ok));

    assert_eq!(manifest.total_rows(), 100);
    println!(
        "manifest ok: table={}, files={}, total_rows={}, total_bytes={}",
        manifest.table,
        manifest.files.len(),
        manifest.total_rows(),
        manifest.total_bytes()
    );
    Ok(())
}
