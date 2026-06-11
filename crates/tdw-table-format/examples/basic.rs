//! Offline table-format manifest round-trip: build an Iceberg `TableManifest`,
//! validate its shape, and verify the SHA-256 content checksum.
//! No network, no docker.
//!
//! Run with: `cargo run -p tdw-table-format --example tdw-table-format-basic`

use std::io::Cursor;

use tdw_table_format::{TableFile, TableFormat, TableManifest, VerifyOutcome};

fn main() -> tdw_table_format::Result<()> {
    let path = "s3://stage/ohlcv.parquet";
    let content = b"parquet-file-bytes-placeholder";

    let file = TableFile::from_reader(path, Cursor::new(content))?;
    let manifest = TableManifest {
        format: TableFormat::Iceberg,
        table: "raw.market_data_bar".to_string(),
        version: 1,
        files: vec![file],
    };

    manifest.validate_shape()?;
    let outcomes = manifest.verify_checksums(|_| Ok(Cursor::new(content)))?;
    assert_eq!(outcomes, vec![VerifyOutcome::Ok]);

    println!(
        "manifest ok: format={:?}, table={}, version={}, files={}",
        manifest.format,
        manifest.table,
        manifest.version,
        manifest.files.len()
    );
    Ok(())
}
