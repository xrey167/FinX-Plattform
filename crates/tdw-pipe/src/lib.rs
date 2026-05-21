#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use tdw_stage::{CopyIntoPlan, StageLocation};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipeDefinition {
    pub name: String,
    pub stage: StageLocation,
    pub target_table: String,
    pub last_offset: u64,
}

impl PipeDefinition {
    pub fn copy_plan(&self, files: Vec<String>) -> CopyIntoPlan {
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
        let plan = pipe.copy_plan(vec!["ohlcv.parquet".to_string()]);
        pipe.advance(42);

        assert_eq!(plan.target_table, "raw.market_data_bar");
        assert_eq!(pipe.last_offset, 42);
    }
}
