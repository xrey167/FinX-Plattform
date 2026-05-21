#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

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

impl CopyIntoPlan {
    pub fn new(stage: StageLocation, target_table: impl Into<String>, files: Vec<String>) -> Self {
        let checksum = files
            .iter()
            .flat_map(|file| file.bytes())
            .map(u64::from)
            .sum();
        Self {
            stage,
            target_table: target_table.into(),
            files,
            checksum,
        }
    }
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
        );

        assert_eq!(plan.target_table, "raw.market_data_bar");
        assert!(plan.checksum > 0);
    }
}
