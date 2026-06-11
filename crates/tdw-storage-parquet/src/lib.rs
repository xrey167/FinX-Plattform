#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]

//! Parquet dataset manifest: file-level and manifest-level content integrity.
//!
//! # Integrity guarantees
//!
//! [`ParquetFile::from_reader`] computes a **SHA-256 digest of the parquet
//! file bytes** via a streaming reader (64 KiB chunks; the file is never fully
//! loaded into memory).  [`ParquetDatasetManifest::verify_checksums`] accepts
//! an `open` callback that produces a reader for each listed path, re-hashes
//! the bytes, and compares against the stored digest.  A corrupted or
//! partially written parquet file is detected.
//!
//! The `(path, row_count, content_length)` triple is kept as **metadata
//! validation** alongside the content hash: shape changes (wrong row count,
//! truncated file length) are caught by
//! [`ParquetDatasetManifest::verify_metadata`] without needing to re-read
//! file bytes.
//!
//! ## What is **not** guaranteed
//!
//! * Remote storage consistency (S3 read-after-write, object versioning).
//! * Atomicity of multi-file commits — each file is verified independently.
//!
//! ## Legacy manifests
//!
//! Manifests written before G008 (2026-06-11) set
//! `content_checksum_kind = "legacy"`.
//! [`ParquetDatasetManifest::verify_checksums`] returns
//! [`VerifyOutcome::LegacyUnverified`] for every such entry without reading
//! the file bytes — this is a **loud, distinct** status that is neither pass
//! nor fail.  Callers should surface it and schedule a re-manifest.

use std::io::{self, Read};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, ParquetManifestError>;

// ---------------------------------------------------------------------------
// ChecksumKind
// ---------------------------------------------------------------------------

/// How the [`ParquetFile::content_checksum`] field was produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumKind {
    /// Checksum is the lower-case hex-encoded SHA-256 of the **file bytes**.
    /// This is the only kind written by current code.
    #[default]
    Sha256,
    /// Checksum was produced by the pre-G008 `parquet_file_checksum` function,
    /// which FNV-hashed `path + row_count + content_length` — there was no
    /// file I/O and the parquet bytes were never read.  The stored value
    /// provides **no** content-integrity guarantee; it is preserved so readers
    /// surface a distinct "legacy, content-unverified" status rather than
    /// silently passing or failing.
    Legacy,
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

/// Manifest over a set of parquet files belonging to one logical table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParquetDatasetManifest {
    pub table: String,
    pub files: Vec<ParquetFile>,
}

/// One parquet file entry inside a [`ParquetDatasetManifest`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParquetFile {
    /// Path or URI to the parquet file.
    pub path: String,
    /// Number of rows in the file — metadata validation only.
    pub row_count: u64,
    /// On-disk byte length of the file — metadata validation only.
    pub content_length: u64,
    /// Algorithm used to produce `content_checksum`.
    #[serde(default)]
    pub content_checksum_kind: ChecksumKind,
    /// Content digest.
    ///
    /// * [`ChecksumKind::Sha256`] — lower-case hex SHA-256 of the file bytes.
    /// * [`ChecksumKind::Legacy`] — decimal FNV of `path+row_count+length`;
    ///   has no content-integrity meaning.
    pub content_checksum: String,
}

// ---------------------------------------------------------------------------
// VerifyOutcome
// ---------------------------------------------------------------------------

/// Per-file result from [`ParquetDatasetManifest::verify_checksums`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// File bytes matched the stored SHA-256 digest.
    Ok,
    /// Entry was written by pre-G008 code.  File bytes were **not** read;
    /// no pass/fail verdict is available.  Schedule a re-manifest.
    LegacyUnverified { path: String },
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

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
    #[error("parquet file {path} content checksum mismatch: expected {expected}, actual {actual}")]
    ContentChecksumMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("parquet file {path} I/O error: {message}")]
    Io { path: String, message: String },
}

// ---------------------------------------------------------------------------
// Impl ParquetDatasetManifest
// ---------------------------------------------------------------------------

impl ParquetDatasetManifest {
    /// Build a manifest and validate shape constraints.
    ///
    /// # Errors
    ///
    /// Returns the first shape violation encountered.
    pub fn new(table: impl Into<String>, files: Vec<ParquetFile>) -> Result<Self> {
        let manifest = Self {
            table: table.into(),
            files,
        };
        manifest.validate_shape()?;
        Ok(manifest)
    }

    /// Sum of all recorded row counts (metadata, not re-validated from disk).
    #[must_use]
    pub fn total_rows(&self) -> u64 {
        self.files.iter().map(|f| f.row_count).sum()
    }

    /// Sum of all recorded content lengths (metadata, not re-validated from
    /// disk).
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.content_length).sum()
    }

    /// Validate shape constraints (non-empty table, files list, paths,
    /// `row_count`, `content_length`).  Does **not** open any files.
    ///
    /// # Errors
    ///
    /// Returns the first shape violation.
    pub fn validate_shape(&self) -> Result<()> {
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

    /// Validate recorded metadata `(row_count, content_length)` for each file
    /// without re-reading file bytes.  `meta` is called with each path and
    /// must return `(actual_row_count, actual_content_length)`.
    ///
    /// This is a cheap pre-flight check; use [`Self::verify_checksums`] for
    /// full content integrity.
    ///
    /// # Errors
    ///
    /// Returns [`ParquetManifestError::EmptyFileRows`] /
    /// [`ParquetManifestError::EmptyFileBytes`] if the recorded values no
    /// longer match what `meta` returns.
    pub fn verify_metadata(&self, mut meta: impl FnMut(&str) -> (u64, u64)) -> Result<()> {
        self.validate_shape()?;
        for file in &self.files {
            let (actual_rows, actual_bytes) = meta(&file.path);
            if actual_rows != file.row_count {
                return Err(ParquetManifestError::EmptyFileRows {
                    path: file.path.clone(),
                });
            }
            if actual_bytes != file.content_length {
                return Err(ParquetManifestError::EmptyFileBytes {
                    path: file.path.clone(),
                });
            }
        }
        Ok(())
    }

    /// Re-read every file via `open` and verify its SHA-256 content digest.
    ///
    /// `open` receives the `path` string and must return a [`Read`] over the
    /// file bytes.  The implementation hashes in 64 KiB chunks and never
    /// loads the whole file into memory.
    ///
    /// Legacy entries (written before G008) return
    /// [`VerifyOutcome::LegacyUnverified`] without calling `open`.
    ///
    /// # Errors
    ///
    /// Returns [`ParquetManifestError::ContentChecksumMismatch`] for the
    /// first detected corruption, or [`ParquetManifestError::Io`] if `open`
    /// fails.
    pub fn verify_checksums<R: Read>(
        &self,
        mut open: impl FnMut(&str) -> io::Result<R>,
    ) -> Result<Vec<VerifyOutcome>> {
        self.validate_shape()?;
        let mut outcomes = Vec::with_capacity(self.files.len());
        for file in &self.files {
            match file.content_checksum_kind {
                ChecksumKind::Legacy => {
                    outcomes.push(VerifyOutcome::LegacyUnverified {
                        path: file.path.clone(),
                    });
                }
                ChecksumKind::Sha256 => {
                    let reader = open(&file.path).map_err(|e| ParquetManifestError::Io {
                        path: file.path.clone(),
                        message: e.to_string(),
                    })?;
                    let actual = sha256_reader(reader).map_err(|e| ParquetManifestError::Io {
                        path: file.path.clone(),
                        message: e.to_string(),
                    })?;
                    if actual != file.content_checksum {
                        return Err(ParquetManifestError::ContentChecksumMismatch {
                            path: file.path.clone(),
                            expected: file.content_checksum.clone(),
                            actual,
                        });
                    }
                    outcomes.push(VerifyOutcome::Ok);
                }
            }
        }
        Ok(outcomes)
    }
}

// ---------------------------------------------------------------------------
// Impl ParquetFile
// ---------------------------------------------------------------------------

impl ParquetFile {
    /// Build a [`ParquetFile`] by computing the SHA-256 digest of the bytes
    /// produced by `reader`.
    ///
    /// `row_count` and `content_length` are recorded as metadata alongside
    /// the content hash.  The reader is consumed in 64 KiB chunks; the file
    /// is never fully loaded into memory.
    ///
    /// # Errors
    ///
    /// Returns a shape violation if `path`/`row_count`/`content_length` are empty,
    /// or [`ParquetManifestError::Io`] if reading fails.
    pub fn from_reader(
        path: impl Into<String>,
        row_count: u64,
        content_length: u64,
        reader: impl Read,
    ) -> Result<Self> {
        let path = path.into();
        let file = Self {
            content_checksum: String::new(), // filled below
            content_checksum_kind: ChecksumKind::Sha256,
            path,
            row_count,
            content_length,
        };
        file.validate_shape()?;
        let digest = sha256_reader(reader).map_err(|e| ParquetManifestError::Io {
            path: file.path.clone(),
            message: e.to_string(),
        })?;
        Ok(Self {
            content_checksum: digest,
            ..file
        })
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

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Stream `reader` through SHA-256 in 64 KiB chunks and return the lower-case
/// hex digest.  Allocates one chunk buffer; never reads the whole file into
/// memory.
fn sha256_reader(mut reader: impl Read) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 65_536];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    Ok(digest.iter().fold(String::with_capacity(64), |mut acc, b| {
        use std::fmt::Write as _;
        let _ = write!(acc, "{b:02x}");
        acc
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn make_file(path: &str, content: &[u8]) -> ParquetFile {
        ParquetFile::from_reader(
            path,
            42,
            content.len() as u64,
            Cursor::new(content.to_vec()),
        )
        .expect("ParquetFile::from_reader should not fail for valid input")
    }

    fn make_manifest(files: Vec<ParquetFile>) -> ParquetDatasetManifest {
        ParquetDatasetManifest::new("raw.market_data_bar", files)
            .expect("ParquetDatasetManifest::new should not fail for valid input")
    }

    // -----------------------------------------------------------------------
    // round-trip: totals correct and content hash verifies
    // -----------------------------------------------------------------------

    #[test]
    fn round_trip_pass() {
        let content = b"parquet-file-bytes-placeholder";
        let file = make_file("s3://bucket/raw/ohlcv.parquet", content);
        let manifest = make_manifest(vec![file]);

        assert_eq!(manifest.total_rows(), 42);
        assert_eq!(manifest.total_bytes(), content.len() as u64);

        let outcomes = manifest
            .verify_checksums(|_| Ok(Cursor::new(content.to_vec())))
            .expect("verify_checksums should pass on unmodified content");
        assert_eq!(outcomes, vec![VerifyOutcome::Ok]);
    }

    // -----------------------------------------------------------------------
    // corrupted-file detection: flip bytes keeping length, verify FAILS
    // -----------------------------------------------------------------------

    #[test]
    fn corrupted_file_detected() {
        let original = b"parquet-file-bytes-placeholder";
        let file = make_file("s3://bucket/raw/ohlcv.parquet", original);
        let manifest = make_manifest(vec![file]);

        let corrupted: Vec<u8> = original.iter().map(|b| b ^ 0xFF).collect();
        assert_eq!(corrupted.len(), original.len(), "length must be preserved");

        let err = manifest
            .verify_checksums(|_| Ok(Cursor::new(corrupted.clone())))
            .expect_err("corrupted file must be detected");
        assert!(
            matches!(err, ParquetManifestError::ContentChecksumMismatch { .. }),
            "expected ContentChecksumMismatch, got {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // legacy manifest: distinct LegacyUnverified status, never opens file
    // -----------------------------------------------------------------------

    #[test]
    fn legacy_manifest_reports_distinct_unverified_status() {
        let file = ParquetFile {
            path: "s3://bucket/raw/ohlcv.parquet".to_string(),
            row_count: 42,
            content_length: 4096,
            content_checksum_kind: ChecksumKind::Legacy,
            // old FNV-of-metadata value — no content-integrity meaning
            content_checksum: "12345678901234567".to_string(),
        };
        let manifest = make_manifest(vec![file.clone()]);

        let outcomes = manifest
            .verify_checksums(|_path| -> io::Result<Cursor<Vec<u8>>> {
                Err(io::Error::other("must not be called for legacy entries"))
            })
            .expect("verify_checksums must not error on legacy entries");

        assert_eq!(
            outcomes,
            vec![VerifyOutcome::LegacyUnverified { path: file.path }]
        );
    }

    // -----------------------------------------------------------------------
    // large-file streaming: 4 MiB, well above the 64 KiB chunk size
    // -----------------------------------------------------------------------

    #[test]
    fn large_file_streaming_no_full_read() {
        let chunk = b"TDW-STREAMING-CHUNK-PATTERN-0123456789ABCDEF";
        let big: Vec<u8> = chunk
            .iter()
            .copied()
            .cycle()
            .take(4 * 1024 * 1024)
            .collect();

        let file = make_file("s3://bucket/raw/large.parquet", &big);
        let manifest = make_manifest(vec![file]);

        let outcomes = manifest
            .verify_checksums(|_| Ok(Cursor::new(big.clone())))
            .expect("large-file round-trip should pass");
        assert_eq!(outcomes, vec![VerifyOutcome::Ok]);
    }

    // -----------------------------------------------------------------------
    // shape validation
    // -----------------------------------------------------------------------

    #[test]
    fn shape_validation_rejects_empty_fields() {
        let content = b"some bytes";
        assert!(
            ParquetDatasetManifest::new("", vec![make_file("s3://x/f.parquet", content)]).is_err()
        );
        assert!(
            ParquetFile::from_reader("", 1, 1, Cursor::new(b"x".to_vec())).is_err(),
            "empty path"
        );
        assert!(
            ParquetFile::from_reader("s3://x/f.parquet", 0, 1, Cursor::new(b"x".to_vec())).is_err(),
            "zero rows"
        );
        assert!(
            ParquetFile::from_reader("s3://x/f.parquet", 1, 0, Cursor::new(b"x".to_vec())).is_err(),
            "zero length"
        );
    }

    // -----------------------------------------------------------------------
    // metadata validation helper
    // -----------------------------------------------------------------------

    #[test]
    fn verify_metadata_detects_row_count_drift() {
        let content = b"data";
        let file = make_file("s3://bucket/raw/ohlcv.parquet", content);
        let expected_rows = file.row_count;
        let expected_bytes = file.content_length;
        let manifest = make_manifest(vec![file]);

        // correct metadata passes
        assert!(
            manifest
                .verify_metadata(|_| (expected_rows, expected_bytes))
                .is_ok()
        );

        // wrong row count fails
        assert!(
            manifest
                .verify_metadata(|_| (expected_rows + 1, expected_bytes))
                .is_err()
        );
    }
}
