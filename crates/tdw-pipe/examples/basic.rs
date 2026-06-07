//! Offline `tdw-pipe` example: define an ingestion pipe, generate a COPY INTO
//! plan for a batch, and advance the monotonic offset cursor.
//!
//! Run with:
//!
//! ```text
//! cargo run -p tdw-pipe --example basic
//! ```

use tdw_pipe::PipeDefinition;
use tdw_stage::StageLocation;

fn main() {
    let mut pipe = PipeDefinition {
        name: "market-pipe".to_string(),
        stage: StageLocation {
            name: "market-stage".to_string(),
            uri: "s3://bucket/market".to_string(),
        },
        target_table: "raw.market_data_bar".to_string(),
        last_offset: 0,
    };

    // Meaningful operation: compose the pipe's stage into a validated load plan.
    let plan = pipe
        .copy_plan(vec!["ohlcv.parquet".to_string()])
        .expect("pipe should produce a valid plan");
    println!(
        "pipe '{}' -> table {} (checksum={})",
        pipe.name, plan.target_table, plan.checksum
    );

    // Advancing the offset is monotonic: an earlier offset never rewinds it.
    pipe.advance(42);
    pipe.advance(7);
    println!(
        "last_offset after advance(42) then advance(7): {}",
        pipe.last_offset
    );
}
