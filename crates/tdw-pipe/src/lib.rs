#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
use serde::{Deserialize, Serialize};
use tdw_stage::{CopyIntoPlan, Result as StageResult, StageLocation};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipeDefinition {
    pub name: String,
    pub stage: StageLocation,
    pub target_table: String,
    pub last_offset: u64,
}

impl PipeDefinition {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn copy_plan(&self, files: Vec<String>) -> StageResult<CopyIntoPlan> {
        CopyIntoPlan::new(self.stage.clone(), self.target_table.clone(), files)
    }

    pub fn advance(&mut self, offset: u64) {
        self.last_offset = self.last_offset.max(offset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_creates_copy_plan_and_advances_offsets() {
        let mut pipe = PipeDefinition {
            name: "market-pipe".to_string(),
            stage: StageLocation {
                name: "market-stage".to_string(),
                uri: "s3://bucket/market".to_string(),
            },
            target_table: "raw.market_data_bar".to_string(),
            last_offset: 0,
        };
        let plan = pipe
            .copy_plan(vec!["ohlcv.parquet".to_string()])
            .unwrap_or_else(|error| panic!("copy plan should be valid: {error}"));
        pipe.advance(42);
        pipe.advance(7);

        assert_eq!(plan.target_table, "raw.market_data_bar");
        assert_eq!(pipe.last_offset, 42);
    }
}
