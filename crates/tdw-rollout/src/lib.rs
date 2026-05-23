#![forbid(unsafe_code)]

use std::fs::{self, OpenOptions};
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

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, record: &RolloutRecord) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        serde_json::to_writer(&mut file, record)?;
        file.write_all(b"\n")?;
        Ok(())
    }

    pub fn read_all(&self) -> Result<Vec<RolloutRecord>> {
        let file = OpenOptions::new().read(true).open(&self.path)?;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tdw_protocol::{EventMsg, OpId, SessionId};

    #[test]
    fn appends_and_reads_jsonl_records() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tdw-rollout-{suffix}.jsonl"));
        let rollout = JsonlRollout::new(&path);
        let record = RolloutRecord {
            recorded_at: "2026-05-22T00:00:00Z".to_string(),
            frame: ReplayFrame {
                session_id: SessionId::new("session-1").expect("session id"),
                sequence: 1,
                event: EventMsg::Started {
                    op_id: OpId::generated(),
                },
            },
        };

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
}
