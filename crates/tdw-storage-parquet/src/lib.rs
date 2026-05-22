#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, ParquetManifestError>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParquetDatasetManifest {
    pub table: String,
    pub files: Vec<ParquetFile>,
    pub checksum: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParquetFile {
    pub path: String,
    pub row_count: u64,
    pub content_length: u64,
    pub checksum: u64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParquetManifestError {
    #[error("parquet table name must not be empty")]
    EmptyTable,
    #[error("parquet manifest must include at least one file")]
    EmptyFiles,
    #[error("parquet file path must not be empty")]
    EmptyFilePath,
    #[error("parquet file {path} must contain at least one row")]
    EmptyFileRows { path: String },
    #[error("parquet file {path} must have a non-zero content length")]
    EmptyFileBytes { path: String },
    #[error("parquet file {path} checksum mismatch: expected {expected}, actual {actual}")]
    FileChecksumMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
    #[error("parquet manifest checksum mismatch: expected {expected}, actual {actual}")]
    ManifestChecksumMismatch { expected: u64, actual: u64 },
}

impl ParquetDatasetManifest {
    pub fn new(table: impl Into<String>, files: Vec<ParquetFile>) -> Result<Self> {
        let mut manifest = Self {
            table: table.into(),
            files,
            checksum: 0,
        };
        manifest.validate_shape()?;
        manifest.checksum = manifest.calculate_checksum();
        Ok(manifest)
    }

    pub fn total_rows(&self) -> u64 {
        self.files.iter().map(|file| file.row_count).sum()
    }

    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.content_length).sum()
    }

    pub fn verify_checksums(&self) -> Result<()> {
        self.validate_shape()?;
        for file in &self.files {
            file.verify_checksum()?;
        }
        let expected = self.calculate_checksum();
        if self.checksum != expected {
            return Err(ParquetManifestError::ManifestChecksumMismatch {
                expected,
                actual: self.checksum,
            });
        }
        Ok(())
    }

    fn validate_shape(&self) -> Result<()> {
        if self.table.trim().is_empty() {
            return Err(ParquetManifestError::EmptyTable);
        }
        if self.files.is_empty() {
            return Err(ParquetManifestError::EmptyFiles);
        }
        for file in &self.files {
            file.validate_shape()?;
        }
        Ok(())
    }

    fn calculate_checksum(&self) -> u64 {
        let mut checksum = checksum_bytes(self.table.as_bytes());
        for file in &self.files {
            checksum = checksum.wrapping_mul(FNV_PRIME).wrapping_add(file.checksum);
        }
        checksum
    }
}

impl ParquetFile {
    pub fn new(path: impl Into<String>, row_count: u64, content_length: u64) -> Result<Self> {
        let path = path.into();
        let file = Self {
            checksum: parquet_file_checksum(&path, row_count, content_length),
            path,
            row_count,
            content_length,
        };
        file.validate_shape()?;
        Ok(file)
    }

    pub fn verify_checksum(&self) -> Result<()> {
        self.validate_shape()?;
        let expected = parquet_file_checksum(&self.path, self.row_count, self.content_length);
        if self.checksum != expected {
            return Err(ParquetManifestError::FileChecksumMismatch {
                path: self.path.clone(),
                expected,
                actual: self.checksum,
            });
        }
        Ok(())
    }

    fn validate_shape(&self) -> Result<()> {
        if self.path.trim().is_empty() {
            return Err(ParquetManifestError::EmptyFilePath);
        }
        if self.row_count == 0 {
            return Err(ParquetManifestError::EmptyFileRows {
                path: self.path.clone(),
            });
        }
        if self.content_length == 0 {
            return Err(ParquetManifestError::EmptyFileBytes {
                path: self.path.clone(),
            });
        }
        Ok(())
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn parquet_file_checksum(path: &str, row_count: u64, content_length: u64) -> u64 {
    let mut checksum = checksum_bytes(path.as_bytes());
    checksum = checksum_bytes_with_seed(&row_count.to_le_bytes(), checksum);
    checksum_bytes_with_seed(&content_length.to_le_bytes(), checksum)
}

fn checksum_bytes(bytes: &[u8]) -> u64 {
    checksum_bytes_with_seed(bytes, FNV_OFFSET)
}

fn checksum_bytes_with_seed(bytes: &[u8], seed: u64) -> u64 {
    bytes.iter().fold(seed, |checksum, byte| {
        (checksum ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_records_totals_and_verifies_checksums() {
        let file = ParquetFile::new("s3://bucket/raw/ohlcv.parquet", 42, 4096)
            .unwrap_or_else(|error| panic!("parquet file should be valid: {error}"));
        let manifest = ParquetDatasetManifest::new("raw.market_data_bar", vec![file])
            .unwrap_or_else(|error| panic!("parquet manifest should be valid: {error}"));

        assert_eq!(manifest.total_rows(), 42);
        assert_eq!(manifest.total_bytes(), 4096);
        assert!(manifest.verify_checksums().is_ok());
    }

    #[test]
    fn rejects_empty_manifest_boundaries_and_checksum_drift() {
        assert!(ParquetDatasetManifest::new("", Vec::new()).is_err());
        assert!(ParquetFile::new("", 1, 1).is_err());
        assert!(ParquetFile::new("s3://bucket/raw/empty.parquet", 0, 1).is_err());

        let mut file = ParquetFile::new("s3://bucket/raw/ohlcv.parquet", 1, 1)
            .unwrap_or_else(|error| panic!("parquet file should be valid: {error}"));
        file.checksum = file.checksum.wrapping_add(1);
        let mut manifest = ParquetDatasetManifest::new("raw.market_data_bar", vec![file])
            .unwrap_or_else(|error| {
                panic!("shape-only manifest construction should pass: {error}")
            });
        manifest.checksum = manifest.checksum.wrapping_add(1);

        assert!(manifest.verify_checksums().is_err());
    }
}
