#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]

//! Parquet dataset manifest: file-level content integrity.
//!
//! # Integrity guarantees
//!
//! [`ParquetFile::from_reader`] computes a **SHA-256 digest of the parquet
//! file bytes** via a streaming reader (64 KiB chunks; the file is never fully
//! loaded into memory).  [`ParquetDatasetManifest::verify_checksums`] accepts
//! an `open` callback that produces a reader for each listed path, re-hashes
//! the bytes, and compares against the stored digest.  A corrupted or
//! partially written parquet file is detected and returned as
//! [`ParquetManifestError::ContentChecksumMismatch`].
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
//! ## Legacy manifests (pre-G008)
//!
//! Manifests written before G008 (2026-06-11) used the schema
//! `{"path":"...","row_count":<u64>,"content_length":<u64>,"checksum":<u64>}`
//! where `checksum` was an FNV hash of `path+row_count+content_length` — no
//! file content was ever read.
//!
//! **Detection rule (serde):** an entry that carries the old `checksum` field
//! (and lacks `content_checksum`) deserialises into
//! `content_checksum_kind = Legacy`.  An entry with `content_checksum`
//! deserialises into `content_checksum_kind = Sha256`.  This is handled by a
//! custom `Deserialize` impl on [`ParquetFile`]; the wire format written by
//! current code always uses `content_checksum`.
//!
//! [`ParquetDatasetManifest::verify_checksums`] returns
//! [`VerifyOutcome::LegacyUnverified`] for every Legacy entry without reading
//! the file bytes — a **loud, distinct** status that is neither pass nor fail.
//! Callers should surface it and schedule a re-manifest.

use std::io::{self, Read};

use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, ParquetManifestError>;

// ---------------------------------------------------------------------------
// ChecksumKind
// ---------------------------------------------------------------------------

/// How the [`ParquetFile::content_checksum`] field was produced.
///
/// **Serde detection rule:** entries that deserialise from the old
/// `{"path":"...","row_count":…,"content_length":…,"checksum":<u64>}` shape
/// are automatically tagged `Legacy`.  Entries with a `content_checksum`
/// field are tagged `Sha256`.  Current code always writes `Sha256`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumKind {
    /// Checksum is the lower-case hex-encoded SHA-256 of the **file bytes**.
    /// This is the only kind written by current code.
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
///
/// # Serde shape (new, written by current code)
///
/// ```json
/// {"path":"s3://…","row_count":42,"content_length":4096,
///  "content_checksum_kind":"sha256","content_checksum":"<hex>"}
/// ```
///
/// # Serde shape (old, pre-G008)
///
/// ```json
/// {"path":"s3://…","row_count":42,"content_length":4096,"checksum":<u64>}
/// ```
///
/// Old entries are accepted on deserialisation and mapped to
/// `content_checksum_kind = Legacy`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ParquetFile {
    /// Path or URI to the parquet file.
    pub path: String,
    /// Number of rows in the file — metadata validation only.
    pub row_count: u64,
    /// On-disk byte length of the file — metadata validation only.
    pub content_length: u64,
    /// Algorithm used to produce `content_checksum`.
    pub content_checksum_kind: ChecksumKind,
    /// Content digest.
    ///
    /// * [`ChecksumKind::Sha256`] — lower-case hex SHA-256 of the file bytes.
    /// * [`ChecksumKind::Legacy`] — decimal FNV of `path+row_count+length`;
    ///   has no content-integrity meaning.
    pub content_checksum: String,
}

/// Wire format accepted on deserialisation.  Handles both the new shape
/// (with `content_checksum`) and the pre-G008 shape (with `checksum: u64`).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParquetFileWire {
    path: String,
    row_count: u64,
    content_length: u64,
    /// New field — present on current-format entries.
    content_checksum: Option<String>,
    /// New field — present on current-format entries; accepted for
    /// round-trip fidelity but not inspected (kind is inferred from whether
    /// `content_checksum` is present).
    #[serde(rename = "content_checksum_kind")]
    _content_checksum_kind: Option<String>,
    /// Old field — present only on pre-G008 entries.
    checksum: Option<u64>,
}

impl<'de> Deserialize<'de> for ParquetFile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let wire = ParquetFileWire::deserialize(deserializer)?;
        match wire.content_checksum {
            // New shape: has content_checksum → Sha256.
            Some(digest) => Ok(Self {
                path: wire.path,
                row_count: wire.row_count,
                content_length: wire.content_length,
                content_checksum_kind: ChecksumKind::Sha256,
                content_checksum: digest,
            }),
            // Old shape: no content_checksum → Legacy.  The `checksum` u64
            // is stored as its decimal string for round-trip fidelity but
            // carries no integrity meaning.
            None => Ok(Self {
                path: wire.path,
                row_count: wire.row_count,
                content_length: wire.content_length,
                content_checksum_kind: ChecksumKind::Legacy,
                content_checksum: wire.checksum.map_or_else(String::new, |v| v.to_string()),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// VerifyOutcome
// ---------------------------------------------------------------------------

/// Per-file result from [`ParquetDatasetManifest::verify_checksums`].
///
/// Corruption (hash mismatch) is reported as
/// [`ParquetManifestError::ContentChecksumMismatch`] in the `Result`, not as
/// a variant here, so that callers cannot accidentally ignore it.
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
    /// Returned by [`ParquetDatasetManifest::verify_metadata`] when the
    /// on-disk row count does not match the manifest record.
    #[error("parquet file {path} row count drift: manifest has {expected}, got {actual}")]
    RowCountDrift {
        path: String,
        expected: u64,
        actual: u64,
    },
    /// Returned by [`ParquetDatasetManifest::verify_metadata`] when the
    /// on-disk byte length does not match the manifest record.
    #[error("parquet file {path} content length drift: manifest has {expected}, got {actual}")]
    ContentLengthDrift {
        path: String,
        expected: u64,
        actual: u64,
    },
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
    /// Returns [`ParquetManifestError::RowCountDrift`] when the on-disk row
    /// count differs from the manifest record, or
    /// [`ParquetManifestError::ContentLengthDrift`] when the byte length
    /// differs.
    pub fn verify_metadata(&self, mut meta: impl FnMut(&str) -> (u64, u64)) -> Result<()> {
        self.validate_shape()?;
        for file in &self.files {
            let (actual_rows, actual_bytes) = meta(&file.path);
            if actual_rows != file.row_count {
                return Err(ParquetManifestError::RowCountDrift {
                    path: file.path.clone(),
                    expected: file.row_count,
                    actual: actual_rows,
                });
            }
            if actual_bytes != file.content_length {
                return Err(ParquetManifestError::ContentLengthDrift {
                    path: file.path.clone(),
                    expected: file.content_length,
                    actual: actual_bytes,
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
    /// Legacy entries (pre-G008, detected by `content_checksum_kind = Legacy`)
    /// return [`VerifyOutcome::LegacyUnverified`] without calling `open`.
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
    /// Returns a shape violation if `path`/`row_count`/`content_length` are
    /// empty, or [`ParquetManifestError::Io`] if reading fails.
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
    // legacy manifest via struct literal: LegacyUnverified, never opens file
    // -----------------------------------------------------------------------

    #[test]
    fn legacy_manifest_reports_distinct_unverified_status() {
        let file = ParquetFile {
            path: "s3://bucket/raw/ohlcv.parquet".to_string(),
            row_count: 42,
            content_length: 4096,
            content_checksum_kind: ChecksumKind::Legacy,
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
    // Finding #1/#2: raw old-format JSON deserializes to Legacy
    // -----------------------------------------------------------------------

    #[test]
    fn raw_old_format_json_deserializes_to_legacy() {
        // Pre-G008 manifest JSON: `checksum` field is a u64 FNV hash,
        // no `content_checksum` field present.
        let json = r#"{
            "table": "raw.market_data_bar",
            "files": [
                {
                    "path": "s3://bucket/raw/ohlcv.parquet",
                    "row_count": 42,
                    "content_length": 4096,
                    "checksum": 12345678901234567
                }
            ]
        }"#;
        let manifest: ParquetDatasetManifest =
            serde_json::from_str(json).expect("old-format JSON must deserialize");

        assert_eq!(manifest.files.len(), 1);
        assert_eq!(
            manifest.files[0].content_checksum_kind,
            ChecksumKind::Legacy
        );
        // verify_checksums must yield LegacyUnverified, never opening the file
        let outcomes = manifest
            .verify_checksums(|_| -> io::Result<Cursor<Vec<u8>>> {
                Err(io::Error::other("must not be called for legacy entries"))
            })
            .expect("old-format manifest must not error in verify_checksums");
        assert_eq!(
            outcomes,
            vec![VerifyOutcome::LegacyUnverified {
                path: "s3://bucket/raw/ohlcv.parquet".to_string()
            }]
        );
    }

    // -----------------------------------------------------------------------
    // Finding #2: mixed manifest — legacy + sha256 entries in one manifest
    // -----------------------------------------------------------------------

    #[test]
    fn mixed_manifest_legacy_and_sha256() {
        let sha256_content = b"real-parquet-bytes";
        let sha256_file = make_file("s3://bucket/raw/new.parquet", sha256_content);

        let legacy_file = ParquetFile {
            path: "s3://bucket/raw/old.parquet".to_string(),
            row_count: 10,
            content_length: 1024,
            content_checksum_kind: ChecksumKind::Legacy,
            content_checksum: "9999999".to_string(),
        };
        let manifest = make_manifest(vec![legacy_file, sha256_file]);

        let outcomes = manifest
            .verify_checksums(|path| {
                if path.ends_with("new.parquet") {
                    Ok(Cursor::new(sha256_content.to_vec()))
                } else {
                    Err(io::Error::other("must not open legacy entry"))
                }
            })
            .expect("mixed manifest must not error");

        assert_eq!(outcomes.len(), 2);
        assert!(
            matches!(&outcomes[0], VerifyOutcome::LegacyUnverified { path } if path == "s3://bucket/raw/old.parquet")
        );
        assert_eq!(outcomes[1], VerifyOutcome::Ok);
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
    // Finding #4: verify_metadata uses proper drift variants
    // -----------------------------------------------------------------------

    #[test]
    fn verify_metadata_detects_drift_with_proper_variants() {
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

        // wrong row count returns RowCountDrift with the expected/actual values
        let err = manifest
            .verify_metadata(|_| (expected_rows + 5, expected_bytes))
            .expect_err("row count drift must be detected");
        assert!(
            matches!(
                err,
                ParquetManifestError::RowCountDrift {
                    expected,
                    actual,
                    ..
                } if expected == expected_rows && actual == expected_rows + 5
            ),
            "expected RowCountDrift, got {err:?}"
        );

        // wrong content length returns ContentLengthDrift
        let err = manifest
            .verify_metadata(|_| (expected_rows, expected_bytes + 1))
            .expect_err("content length drift must be detected");
        assert!(
            matches!(
                err,
                ParquetManifestError::ContentLengthDrift {
                    expected,
                    actual,
                    ..
                } if expected == expected_bytes && actual == expected_bytes + 1
            ),
            "expected ContentLengthDrift, got {err:?}"
        );
    }
}
