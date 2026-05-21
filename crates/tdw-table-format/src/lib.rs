#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TableFormat {
    Iceberg,
    Delta,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableFile {
    pub path: String,
    pub checksum: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableManifest {
    pub format: TableFormat,
    pub table: String,
    pub version: u64,
    pub files: Vec<TableFile>,
}

impl TableManifest {
    pub fn verify_checksums(&self) -> bool {
        self.files
            .iter()
            .all(|file| file.checksum == simple_checksum(&file.path))
    }
}

pub fn simple_checksum(value: &str) -> u64 {
    value.bytes().map(u64::from).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_iceberg_and_delta_manifest_checksums() {
        for format in [TableFormat::Iceberg, TableFormat::Delta] {
            let file = TableFile {
                path: "s3://stage/ohlcv.parquet".to_string(),
                checksum: simple_checksum("s3://stage/ohlcv.parquet"),
            };
            let manifest = TableManifest {
                format,
                table: "raw.market_data_bar".to_string(),
                version: 1,
                files: vec![file],
            };
            assert!(manifest.verify_checksums());
        }
    }
}
