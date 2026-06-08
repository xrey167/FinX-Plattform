#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, TableManifestError>;

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

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TableManifestError {
    #[error("table manifest table name must not be empty")]
    EmptyTable,
    #[error("table manifest version must be greater than zero")]
    EmptyVersion,
    #[error("table manifest must include at least one file")]
    EmptyFiles,
    #[error("table manifest file path must not be empty")]
    EmptyFilePath,
    #[error("table manifest checksum mismatch for {path}: expected {expected}, actual {actual}")]
    ChecksumMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
}

impl TableManifest {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn validate(&self) -> Result<()> {
        if self.table.trim().is_empty() {
            return Err(TableManifestError::EmptyTable);
        }
        if self.version == 0 {
            return Err(TableManifestError::EmptyVersion);
        }
        if self.files.is_empty() {
            return Err(TableManifestError::EmptyFiles);
        }
        for file in &self.files {
            if file.path.trim().is_empty() {
                return Err(TableManifestError::EmptyFilePath);
            }
            let expected = simple_checksum(&file.path);
            if file.checksum != expected {
                return Err(TableManifestError::ChecksumMismatch {
                    path: file.path.clone(),
                    expected,
                    actual: file.checksum,
                });
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn verify_checksums(&self) -> bool {
        self.validate().is_ok()
    }
}

#[must_use]
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
            assert!(manifest.validate().is_ok());
        }
    }

    #[test]
    fn rejects_invalid_manifest_shape_and_checksum_drift() {
        let invalid = TableManifest {
            format: TableFormat::Iceberg,
            table: String::new(),
            version: 0,
            files: Vec::new(),
        };
        assert!(invalid.validate().is_err());

        let drifted = TableManifest {
            format: TableFormat::Delta,
            table: "raw.market_data_bar".to_string(),
            version: 1,
            files: vec![TableFile {
                path: "s3://stage/ohlcv.parquet".to_string(),
                checksum: 1,
            }],
        };
        assert!(drifted.validate().is_err());
        assert!(!drifted.verify_checksums());
    }

    #[test]
    fn validate_reports_the_specific_error_variant() {
        let path = "s3://stage/ohlcv.parquet".to_string();
        let base = TableManifest {
            format: TableFormat::Delta,
            table: "raw.market_data_bar".to_string(),
            version: 1,
            files: vec![TableFile {
                path: path.clone(),
                checksum: simple_checksum(&path),
            }],
        };

        // ChecksumMismatch carries the offending path and both checksums.
        let drifted = TableManifest {
            files: vec![TableFile {
                path: path.clone(),
                checksum: 1,
            }],
            ..base.clone()
        };
        assert_eq!(
            drifted.validate(),
            Err(TableManifestError::ChecksumMismatch {
                path: path.clone(),
                expected: simple_checksum(&path),
                actual: 1,
            })
        );

        // Each shape violation maps to its own dedicated variant.
        assert_eq!(
            TableManifest {
                table: String::new(),
                ..base.clone()
            }
            .validate(),
            Err(TableManifestError::EmptyTable)
        );
        assert_eq!(
            TableManifest {
                version: 0,
                ..base.clone()
            }
            .validate(),
            Err(TableManifestError::EmptyVersion)
        );
        assert_eq!(
            TableManifest {
                files: Vec::new(),
                ..base.clone()
            }
            .validate(),
            Err(TableManifestError::EmptyFiles)
        );
        assert_eq!(
            TableManifest {
                files: vec![TableFile {
                    path: "   ".to_string(),
                    checksum: 0,
                }],
                ..base
            }
            .validate(),
            Err(TableManifestError::EmptyFilePath)
        );
    }
}
