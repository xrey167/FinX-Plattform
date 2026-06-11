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
//! is detected and returned as [`TableManifestError::ChecksumMismatch`].
//!
//! ## What is **not** guaranteed
//!
//! * Remote storage consistency (S3 read-after-write, object versioning).
//! * Atomicity of multi-file commits — each file is verified independently.
//!
//! ## Legacy manifests (pre-G008)
//!
//! Manifests written before G008 (2026-06-11) used the schema
//! `{"path":"...", "checksum": <u64>}` where `checksum` was the byte-sum of
//! the path string — no file content was ever read.
//!
//! **Detection rule (serde):** an entry that carries the old `checksum` field
//! (and lacks `content_checksum`) deserialises into `checksum_kind = Legacy`.
//! An entry with `content_checksum` deserialises into `checksum_kind = Sha256`.
//! This is handled by a custom `Deserialize` impl on [`TableFile`]; the wire
//! format written by current code always uses `content_checksum`.
//!
//! [`TableManifest::verify_checksums`] returns [`VerifyOutcome::LegacyUnverified`]
//! for every Legacy entry without reading the file bytes — a **loud, distinct**
//! status that is neither pass nor fail.  Callers should surface it and
//! schedule a re-manifest with real content hashing.

use std::io::{self, Read};

use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, TableManifestError>;

// ---------------------------------------------------------------------------
// ChecksumKind
// ---------------------------------------------------------------------------

/// How the [`TableFile::content_checksum`] field was produced.
///
/// **Serde detection rule:** entries that deserialise from the old
/// `{"path":"...","checksum":<u64>}` shape are automatically tagged
/// `Legacy`.  Entries with a `content_checksum` field are tagged `Sha256`.
/// Current code always writes `Sha256`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumKind {
    /// Checksum is the lower-case hex-encoded SHA-256 of the **file bytes**.
    /// This is the only kind written by current code.
    Sha256,
    /// Checksum was produced by the pre-G008 `simple_checksum` function, which
    /// summed the **path bytes** rather than the file contents.  The stored
    /// value provides **no** data-integrity guarantee; it is preserved so that
    /// readers surface a distinct "legacy, content-unverified" status rather
    /// than silently passing or failing.
    Legacy,
}

// ---------------------------------------------------------------------------
// TableFile
// ---------------------------------------------------------------------------

/// One file entry inside a [`TableManifest`].
///
/// # Serde shape (new, written by current code)
///
/// ```json
/// {"path":"s3://…","checksum_kind":"sha256","content_checksum":"<hex>"}
/// ```
///
/// # Serde shape (old, pre-G008)
///
/// ```json
/// {"path":"s3://…","checksum":<u64>}
/// ```
///
/// Old entries are accepted on deserialisation and mapped to
/// `checksum_kind = Legacy`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TableFile {
    /// Path or URI to the data file (e.g. `s3://stage/ohlcv.parquet`).
    pub path: String,
    /// Algorithm used to produce `content_checksum`.
    pub checksum_kind: ChecksumKind,
    /// Content digest.
    ///
    /// * [`ChecksumKind::Sha256`] — lower-case hex SHA-256 of the file bytes.
    /// * [`ChecksumKind::Legacy`] — decimal representation of the old
    ///   path-byte-sum; has no integrity meaning.
    pub content_checksum: String,
}

/// Wire format accepted on deserialisation.  Handles both the new shape
/// (with `content_checksum`) and the pre-G008 shape (with `checksum: u64`).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TableFileWire {
    path: String,
    /// New field — present on current-format entries.
    content_checksum: Option<String>,
    /// New field — present on current-format entries; accepted for
    /// round-trip fidelity but not inspected (kind is inferred from whether
    /// `content_checksum` is present).
    #[serde(rename = "checksum_kind")]
    _checksum_kind: Option<String>,
    /// Old field — present only on pre-G008 entries.
    checksum: Option<u64>,
}

impl<'de> Deserialize<'de> for TableFile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let wire = TableFileWire::deserialize(deserializer)?;
        match wire.content_checksum {
            // New shape: has content_checksum → Sha256 regardless of
            // checksum_kind field (forward-compat: unknown kind values would
            // be a future extension, not a legacy entry).
            Some(digest) => Ok(Self {
                path: wire.path,
                checksum_kind: ChecksumKind::Sha256,
                content_checksum: digest,
            }),
            // Old shape: no content_checksum → Legacy.  The `checksum` u64
            // is stored as its decimal string for round-trip fidelity but
            // carries no integrity meaning.
            None => Ok(Self {
                path: wire.path,
                checksum_kind: ChecksumKind::Legacy,
                content_checksum: wire.checksum.map_or_else(String::new, |v| v.to_string()),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// TableManifest / TableFormat
// ---------------------------------------------------------------------------

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
///
/// Corruption (hash mismatch) is reported as
/// [`TableManifestError::ChecksumMismatch`] in the `Result`, not as a variant
/// here, so that callers cannot accidentally ignore it.
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
// Impl TableManifest
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
    /// Legacy entries (pre-G008, detected by `checksum_kind = Legacy`) return
    /// [`VerifyOutcome::LegacyUnverified`] without calling `open` — a distinct
    /// status, neither pass nor fail.
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

// ---------------------------------------------------------------------------
// Impl TableFile
// ---------------------------------------------------------------------------

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
    // legacy manifest via struct literal: LegacyUnverified, never opens file
    // -----------------------------------------------------------------------

    #[test]
    fn legacy_manifest_reports_distinct_unverified_status() {
        let file = TableFile {
            path: "s3://stage/ohlcv.parquet".to_string(),
            checksum_kind: ChecksumKind::Legacy,
            content_checksum: "2847".to_string(),
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
        // Pre-G008 manifest JSON: `checksum` field is a u64 path-byte-sum,
        // no `content_checksum` field present.
        let json = r#"{
            "format": "Iceberg",
            "table": "raw.market_data_bar",
            "version": 1,
            "files": [
                {"path": "s3://stage/ohlcv.parquet", "checksum": 2847}
            ]
        }"#;
        let manifest: TableManifest =
            serde_json::from_str(json).expect("old-format JSON must deserialize");

        assert_eq!(manifest.files.len(), 1);
        assert_eq!(manifest.files[0].checksum_kind, ChecksumKind::Legacy);
        // verify_checksums must yield LegacyUnverified, never opening the file
        let outcomes = manifest
            .verify_checksums(|_| -> io::Result<Cursor<Vec<u8>>> {
                Err(io::Error::other("must not be called for legacy entries"))
            })
            .expect("old-format manifest must not error in verify_checksums");
        assert_eq!(
            outcomes,
            vec![VerifyOutcome::LegacyUnverified {
                path: "s3://stage/ohlcv.parquet".to_string()
            }]
        );
    }

    // -----------------------------------------------------------------------
    // Finding #2: mixed manifest — legacy + sha256 entries in one manifest
    // -----------------------------------------------------------------------

    #[test]
    fn mixed_manifest_legacy_and_sha256() {
        let sha256_content = b"real-file-content";
        let sha256_file = file_from_bytes("s3://stage/new.parquet", sha256_content);

        // Build a manifest with one old-format entry injected alongside a new one.
        let legacy_file = TableFile {
            path: "s3://stage/old.parquet".to_string(),
            checksum_kind: ChecksumKind::Legacy,
            content_checksum: "9999".to_string(),
        };
        let manifest = make_manifest(vec![legacy_file, sha256_file]);

        let outcomes = manifest
            .verify_checksums(|path| {
                if path.ends_with("new.parquet") {
                    Ok(Cursor::new(sha256_content.to_vec()))
                } else {
                    // legacy path must never reach here
                    Err(io::Error::other("must not open legacy entry"))
                }
            })
            .expect("mixed manifest must not error");

        assert_eq!(outcomes.len(), 2);
        assert!(
            matches!(&outcomes[0], VerifyOutcome::LegacyUnverified { path } if path == "s3://stage/old.parquet")
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
