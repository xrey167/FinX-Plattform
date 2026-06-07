//! Offline `tdw-stage` example: build a validated COPY INTO plan, then show how
//! checksum drift is caught by re-validation.
//!
//! Run with:
//!
//! ```text
//! cargo run -p tdw-stage --example basic
//! ```

use tdw_stage::{CopyIntoPlan, StageLocation};

fn main() {
    // Meaningful operation: construct + validate a load plan from inline data.
    let mut plan = CopyIntoPlan::new(
        StageLocation {
            name: "market-stage".to_string(),
            uri: "s3://bucket/market".to_string(),
        },
        "raw.market_data_bar",
        vec!["ohlcv.parquet".to_string()],
    )
    .expect("plan should be valid");

    println!(
        "plan: {} <- {:?} (checksum={})",
        plan.target_table, plan.files, plan.checksum
    );
    println!("revalidates clean: {}", plan.validate().is_ok());

    // Tampering with the checksum is detected on the next validate().
    plan.checksum = plan.checksum.wrapping_add(1);
    println!("after checksum drift, valid? {}", plan.validate().is_ok());
}
