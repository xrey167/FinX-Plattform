#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, StagePlanError>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageLocation {
    pub name: String,
    pub uri: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyIntoPlan {
    pub stage: StageLocation,
    pub target_table: String,
    pub files: Vec<String>,
    pub checksum: u64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StagePlanError {
    #[error("stage name must not be empty")]
    EmptyStageName,
    #[error("stage uri must not be empty")]
    EmptyStageUri,
    #[error("copy target table must not be empty")]
    EmptyTargetTable,
    #[error("copy plan must include at least one file")]
    EmptyFiles,
    #[error("copy plan file path must not be empty")]
    EmptyFilePath,
    #[error("copy plan checksum mismatch: expected {expected}, actual {actual}")]
    ChecksumMismatch { expected: u64, actual: u64 },
}

impl CopyIntoPlan {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn new(
        stage: StageLocation,
        target_table: impl Into<String>,
        files: Vec<String>,
    ) -> Result<Self> {
        let plan = Self {
            stage,
            target_table: target_table.into(),
            checksum: calculate_checksum(&files),
            files,
        };
        plan.validate()?;
        Ok(plan)
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn validate(&self) -> Result<()> {
        if self.stage.name.trim().is_empty() {
            return Err(StagePlanError::EmptyStageName);
        }
        if self.stage.uri.trim().is_empty() {
            return Err(StagePlanError::EmptyStageUri);
        }
        if self.target_table.trim().is_empty() {
            return Err(StagePlanError::EmptyTargetTable);
        }
        if self.files.is_empty() {
            return Err(StagePlanError::EmptyFiles);
        }
        if self.files.iter().any(|file| file.trim().is_empty()) {
            return Err(StagePlanError::EmptyFilePath);
        }
        let expected = calculate_checksum(&self.files);
        if self.checksum != expected {
            return Err(StagePlanError::ChecksumMismatch {
                expected,
                actual: self.checksum,
            });
        }
        Ok(())
    }
}

fn calculate_checksum(files: &[String]) -> u64 {
    files
        .iter()
        .flat_map(|file| file.bytes())
        .map(u64::from)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_into_plan_records_stage_target_files_and_checksum() {
        let plan = CopyIntoPlan::new(
            StageLocation {
                name: "market-stage".to_string(),
                uri: "s3://bucket/market".to_string(),
            },
            "raw.market_data_bar",
            vec!["ohlcv.parquet".to_string()],
        )
        .unwrap_or_else(|error| panic!("copy plan should be valid: {error}"));

        assert_eq!(plan.target_table, "raw.market_data_bar");
        assert!(plan.checksum > 0);
        assert!(plan.validate().is_ok());
    }

    #[test]
    fn rejects_invalid_copy_plan_boundaries_and_checksum_drift() {
        assert!(
            CopyIntoPlan::new(
                StageLocation {
                    name: String::new(),
                    uri: "s3://bucket/market".to_string(),
                },
                "raw.market_data_bar",
                vec!["ohlcv.parquet".to_string()],
            )
            .is_err()
        );
        assert!(
            CopyIntoPlan::new(
                StageLocation {
                    name: "market-stage".to_string(),
                    uri: "s3://bucket/market".to_string(),
                },
                "raw.market_data_bar",
                Vec::new(),
            )
            .is_err()
        );

        let mut plan = CopyIntoPlan::new(
            StageLocation {
                name: "market-stage".to_string(),
                uri: "s3://bucket/market".to_string(),
            },
            "raw.market_data_bar",
            vec!["ohlcv.parquet".to_string()],
        )
        .unwrap_or_else(|error| panic!("copy plan should be valid: {error}"));
        plan.checksum = plan.checksum.wrapping_add(1);

        assert!(plan.validate().is_err());
    }

    #[test]
    fn new_reports_each_remaining_boundary_error() {
        let ok_stage = || StageLocation {
            name: "market-stage".to_string(),
            uri: "s3://bucket/market".to_string(),
        };

        assert_eq!(
            CopyIntoPlan::new(
                StageLocation {
                    uri: "  ".to_string(),
                    ..ok_stage()
                },
                "raw.market_data_bar",
                vec!["ohlcv.parquet".to_string()],
            ),
            Err(StagePlanError::EmptyStageUri)
        );
        assert_eq!(
            CopyIntoPlan::new(ok_stage(), "  ", vec!["ohlcv.parquet".to_string()]),
            Err(StagePlanError::EmptyTargetTable)
        );
        assert_eq!(
            CopyIntoPlan::new(ok_stage(), "raw.market_data_bar", vec!["   ".to_string()]),
            Err(StagePlanError::EmptyFilePath)
        );
    }
}
