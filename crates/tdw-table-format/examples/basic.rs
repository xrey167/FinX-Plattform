//! Offline table-format manifest round-trip: build an Iceberg `TableManifest`,
//! validate it, and verify its checksums. No network, no docker.
//!
//! Run with: `cargo run -p tdw-table-format --example tdw-table-format-basic`

use tdw_table_format::{TableFile, TableFormat, TableManifest, simple_checksum};

fn main() -> tdw_table_format::Result<()> {
    let path = "s3://stage/ohlcv.parquet";
    let manifest = TableManifest {
        format: TableFormat::Iceberg,
        table: "raw.market_data_bar".to_string(),
        version: 1,
        files: vec![TableFile {
            path: path.to_string(),
            checksum: simple_checksum(path),
        }],
    };

    manifest.validate()?;
    assert!(manifest.verify_checksums());

    println!(
        "manifest ok: format={:?}, table={}, version={}, files={}",
        manifest.format,
        manifest.table,
        manifest.version,
        manifest.files.len()
    );
    Ok(())
}
