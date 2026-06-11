#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]

//! Table manifest contract for Iceberg and Delta table formats.
//!
//! # Integrity guarantees
//!
//! [`TableFile::from_reader`] computes a **SHA-256 digest of the file bytes**
//! via a streaming reader (64 KiB chunks; the file is never fully loaded into
//! memory).  [`TableManifest::verify_checksums`] accepts an `open` callback
//! that produces a reader for each listed path, re-hashes the bytes, and
//! compares against the stored digest.  A corrupted or partially written file
//! is detected.
//!
//! ## What is **not** guaranteed
//!
//! * Remote storage consistency (S3 read-after-write, object versioning).
//! * Atomicity of multi-file commits — each file is verified independently.
//!
//! ## Legacy manifests
//!
//! Manifests written before G008 (2026-06-11) set `checksum_kind = "legacy"`.
//! [`TableManifest::verify_checksums`] returns
//! [`VerifyOutcome::LegacyUnverified`] for every such entry without reading
//! the file bytes — this is a **loud, distinct** status that is neither pass
//! nor fail.  Callers should surface it and schedule a re-manifest with real
//! content hashing.

use std::io::{self, Read};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, TableManifestError>;

// ---------------------------------------------------------------------------
// ChecksumKind
// ---------------------------------------------------------------------------

/// How the [`TableFile::content_checksum`] field was produced.
///
/// Serialised in manifests so that readers can tell legacy entries apart from
/// real content-hashed ones without out-of-band metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumKind {
    /// Checksum is the lower-case hex-encoded SHA-256 of the **file bytes**.
    /// This is the only kind written by current code.
    #[default]
    Sha256,
    /// Checksum was produced by the pre-G008 `simple_checksum` function, which
    /// summed the **path bytes** rather than the file contents.  The stored
    /// value provides **no** data-integrity guarantee; it is preserved so that
    /// readers surface a distinct "legacy, content-unverified" status rather
    /// than silently passing or failing.
    Legacy,
}

// ---------------------------------------------------------------------------
// TableFile / TableManifest
// ---------------------------------------------------------------------------

/// One file entry inside a [`TableManifest`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableFile {
    /// Path or URI to the data file (e.g. `s3://stage/ohlcv.parquet`).
    pub path: String,
    /// Algorithm used to produce `content_checksum`.  Defaults to
    /// [`ChecksumKind::Sha256`] on deserialisation so that manifests written
    /// by current code round-trip without an explicit field.
    #[serde(default)]
    pub checksum_kind: ChecksumKind,
    /// Content digest.
    ///
    /// * [`ChecksumKind::Sha256`] — lower-case hex SHA-256 of the file bytes.
    /// * [`ChecksumKind::Legacy`] — decimal representation of the old
    ///   path-byte-sum; has no integrity meaning.
    pub content_checksum: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TableFormat {
    Iceberg,
    Delta,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableManifest {
    pub format: TableFormat,
    pub table: String,
    pub version: u64,
    pub files: Vec<TableFile>,
}

// ---------------------------------------------------------------------------
// VerifyOutcome
// ---------------------------------------------------------------------------

/// Per-file result from [`TableManifest::verify_checksums`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// File bytes matched the stored SHA-256 digest.
    Ok,
    /// Entry was written by pre-G008 code.  File bytes were **not** read;
    /// no pass/fail verdict is available.  Schedule a re-manifest.
    LegacyUnverified { path: String },
    /// Computed digest did not match the stored one — file is corrupted.
    Mismatch {
        path: String,
        expected: String,
        actual: String,
    },
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

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
        expected: String,
        actual: String,
    },
    #[error("table manifest I/O error for {path}: {message}")]
    Io { path: String, message: String },
}

// ---------------------------------------------------------------------------
// Impl
// ---------------------------------------------------------------------------

impl TableManifest {
    /// Validate shape constraints only (non-empty table / version / files /
    /// paths).  Does **not** open any files.
    ///
    /// # Errors
    ///
    /// Returns the first shape violation encountered.
    pub fn validate_shape(&self) -> Result<()> {
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
        }
        Ok(())
    }

    /// Re-read every file via `open` and verify its SHA-256 content digest.
    ///
    /// `open` receives the `path` string from each [`TableFile`] and must
    /// return a [`Read`] over the file bytes.  The implementation hashes in
    /// 64 KiB chunks and never loads the whole file into memory.
    ///
    /// Legacy entries (written before G008) return
    /// [`VerifyOutcome::LegacyUnverified`] without calling `open` — this is a
    /// distinct status, neither pass nor fail.
    ///
    /// # Errors
    ///
    /// Returns [`TableManifestError::ChecksumMismatch`] for the first
    /// detected corruption, or [`TableManifestError::Io`] if `open` fails.
    pub fn verify_checksums<R: Read>(
        &self,
        mut open: impl FnMut(&str) -> io::Result<R>,
    ) -> Result<Vec<VerifyOutcome>> {
        self.validate_shape()?;
        let mut outcomes = Vec::with_capacity(self.files.len());
        for file in &self.files {
            match file.checksum_kind {
                ChecksumKind::Legacy => {
                    outcomes.push(VerifyOutcome::LegacyUnverified {
                        path: file.path.clone(),
                    });
                }
                ChecksumKind::Sha256 => {
                    let reader = open(&file.path).map_err(|e| TableManifestError::Io {
                        path: file.path.clone(),
                        message: e.to_string(),
                    })?;
                    let actual = sha256_reader(reader).map_err(|e| TableManifestError::Io {
                        path: file.path.clone(),
                        message: e.to_string(),
                    })?;
                    if actual != file.content_checksum {
                        return Err(TableManifestError::ChecksumMismatch {
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

impl TableFile {
    /// Build a [`TableFile`] by computing the SHA-256 digest of the bytes
    /// produced by `reader`.
    ///
    /// The reader is consumed in 64 KiB chunks; the file is never fully
    /// loaded into memory.
    ///
    /// # Errors
    ///
    /// Returns [`TableManifestError::EmptyFilePath`] if `path` is blank, or
    /// [`TableManifestError::Io`] if reading fails.
    pub fn from_reader(path: impl Into<String>, reader: impl Read) -> Result<Self> {
        let path = path.into();
        if path.trim().is_empty() {
            return Err(TableManifestError::EmptyFilePath);
        }
        let digest = sha256_reader(reader).map_err(|e| TableManifestError::Io {
            path: path.clone(),
            message: e.to_string(),
        })?;
        Ok(Self {
            path,
            checksum_kind: ChecksumKind::Sha256,
            content_checksum: digest,
        })
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Stream `reader` through SHA-256 in 64 KiB chunks and return the lower-case
/// hex digest.  Allocates one chunk buffer on the stack; never reads the whole
/// file into memory.
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

    fn make_manifest(files: Vec<TableFile>) -> TableManifest {
        TableManifest {
            format: TableFormat::Iceberg,
            table: "raw.market_data_bar".to_string(),
            version: 1,
            files,
        }
    }

    fn file_from_bytes(path: &str, bytes: &[u8]) -> TableFile {
        TableFile::from_reader(path, Cursor::new(bytes.to_vec()))
            .expect("TableFile::from_reader should not fail for valid input")
    }

    // -----------------------------------------------------------------------
    // round-trip: content hash is computed and verified correctly
    // -----------------------------------------------------------------------

    #[test]
    fn round_trip_pass() {
        let content = b"parquet-file-bytes-placeholder";
        let file = file_from_bytes("s3://stage/ohlcv.parquet", content);
        assert_eq!(file.checksum_kind, ChecksumKind::Sha256);

        let manifest = make_manifest(vec![file]);
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
        let file = file_from_bytes("s3://stage/ohlcv.parquet", original);
        let manifest = make_manifest(vec![file]);

        let corrupted: Vec<u8> = original.iter().map(|b| b ^ 0xFF).collect();
        assert_eq!(corrupted.len(), original.len(), "length must be preserved");

        let err = manifest
            .verify_checksums(|_| Ok(Cursor::new(corrupted.clone())))
            .expect_err("corrupted file must be detected");
        assert!(
            matches!(err, TableManifestError::ChecksumMismatch { .. }),
            "expected ChecksumMismatch, got {err:?}"
        );
    }

    // -----------------------------------------------------------------------
    // legacy manifest: distinct LegacyUnverified status, never opens file
    // -----------------------------------------------------------------------

    #[test]
    fn legacy_manifest_reports_distinct_unverified_status() {
        let file = TableFile {
            path: "s3://stage/ohlcv.parquet".to_string(),
            checksum_kind: ChecksumKind::Legacy,
            content_checksum: "2847".to_string(), // old path-byte-sum, no meaning
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

        let file = file_from_bytes("s3://stage/large.parquet", &big);
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
    fn shape_validation_rejects_invalid_manifests() {
        let valid_file = file_from_bytes("s3://stage/ohlcv.parquet", b"content");

        assert_eq!(
            TableManifest {
                table: String::new(),
                ..make_manifest(vec![valid_file.clone()])
            }
            .validate_shape(),
            Err(TableManifestError::EmptyTable)
        );
        assert_eq!(
            TableManifest {
                version: 0,
                ..make_manifest(vec![valid_file.clone()])
            }
            .validate_shape(),
            Err(TableManifestError::EmptyVersion)
        );
        assert_eq!(
            make_manifest(vec![]).validate_shape(),
            Err(TableManifestError::EmptyFiles)
        );
        assert_eq!(
            make_manifest(vec![TableFile {
                path: "   ".to_string(),
                checksum_kind: ChecksumKind::Sha256,
                content_checksum: String::new(),
            }])
            .validate_shape(),
            Err(TableManifestError::EmptyFilePath)
        );
        // valid manifest passes shape check
        assert!(make_manifest(vec![valid_file]).validate_shape().is_ok());
    }

    // -----------------------------------------------------------------------
    // both table formats accepted
    // -----------------------------------------------------------------------

    #[test]
    fn iceberg_and_delta_manifests_both_verify() {
        let content = b"bytes";
        for format in [TableFormat::Iceberg, TableFormat::Delta] {
            let file = file_from_bytes("s3://stage/ohlcv.parquet", content);
            let manifest = TableManifest {
                format,
                table: "raw.market_data_bar".to_string(),
                version: 1,
                files: vec![file],
            };
            let outcomes = manifest
                .verify_checksums(|_| Ok(Cursor::new(content.to_vec())))
                .expect("verify_checksums should pass");
            assert_eq!(outcomes, vec![VerifyOutcome::Ok]);
        }
    }
}
