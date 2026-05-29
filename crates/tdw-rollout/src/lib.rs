#![forbid(unsafe_code)]

#[cfg(feature = "postgres")]
pub mod pg_rollout;

#[cfg(feature = "postgres")]
pub use pg_rollout::PgRollout;

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tdw_protocol::ReplayFrame;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, RolloutError>;

#[derive(Debug, Error)]
pub enum RolloutError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RolloutRecord {
    pub recorded_at: String,
    pub frame: ReplayFrame,
}

#[derive(Clone, Debug)]
pub struct JsonlRollout {
    path: PathBuf,
}

impl JsonlRollout {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn append(&self, record: &RolloutRecord) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&self.path)?;
        file.lock()?;
        serde_json::to_writer(&mut file, record)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn read_all(&self) -> Result<Vec<RolloutRecord>> {
        let file = OpenOptions::new().read(true).open(&self.path)?;
        file.lock_shared()?;
        read_records(file)
    }
}

fn read_records(file: File) -> Result<Vec<RolloutRecord>> {
    BufReader::new(file)
        .lines()
        .filter_map(|line| match line {
            Ok(line) if line.trim().is_empty() => None,
            other => Some(other),
        })
        .map(|line| {
            let line = line?;
            serde_json::from_str(&line).map_err(Into::into)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tdw_protocol::{EventMsg, OpId, SessionId};

    fn unique_path(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}-{suffix}.jsonl"))
    }

    fn record(sequence: u64) -> RolloutRecord {
        RolloutRecord {
            recorded_at: "2026-05-22T00:00:00Z".to_string(),
            frame: ReplayFrame {
                session_id: SessionId::new("session-1").expect("session id"),
                sequence,
                event: EventMsg::Started {
                    op_id: OpId::generated(),
                },
            },
        }
    }

    #[test]
    fn appends_and_reads_jsonl_records() {
        let path = unique_path("tdw-rollout");
        let rollout = JsonlRollout::new(&path);
        let record = record(1);

        rollout
            .append(&record)
            .unwrap_or_else(|error| panic!("append succeeds: {error}"));
        rollout
            .append(&record)
            .unwrap_or_else(|error| panic!("append succeeds: {error}"));
        let records = rollout
            .read_all()
            .unwrap_or_else(|error| panic!("read succeeds: {error}"));

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].frame.sequence, 1);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn concurrent_appends_are_serialized_by_file_lock() {
        let path = unique_path("tdw-rollout-concurrent");
        let rollout = Arc::new(JsonlRollout::new(&path));
        let mut handles = Vec::new();
        for sequence in 0..16 {
            let rollout = Arc::clone(&rollout);
            handles.push(thread::spawn(move || {
                rollout
                    .append(&record(sequence))
                    .unwrap_or_else(|error| panic!("append {sequence}: {error}"));
            }));
        }
        for handle in handles {
            handle
                .join()
                .unwrap_or_else(|error| panic!("writer thread panicked: {error:?}"));
        }

        let records = rollout
            .read_all()
            .unwrap_or_else(|error| panic!("read succeeds: {error}"));
        let mut sequences = records
            .iter()
            .map(|record| record.frame.sequence)
            .collect::<Vec<_>>();
        sequences.sort_unstable();

        assert_eq!(sequences, (0..16).collect::<Vec<_>>());
        let _ = fs::remove_file(path);
    }
}
