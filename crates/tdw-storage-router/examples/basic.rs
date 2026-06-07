//! Offline `StorageRouter` round-trip: fan one batch out to two recording sinks
//! and observe the summed receipt. No network, no docker.
//!
//! Run with: `cargo run -p tdw-storage-router --example tdw-storage-router-basic`

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tdw_core::{OBBject, WriteSink};
use tdw_storage_router::{RecordingSink, StorageRouter};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
struct Row {
    symbol: String,
}

#[tokio::main]
async fn main() -> tdw_core::Result<()> {
    let primary = Arc::new(RecordingSink::new("primary"));
    let replica = Arc::new(RecordingSink::new("replica"));

    let mut router = StorageRouter::<Row>::new();
    router.add_sink(primary.clone());
    router.add_sink(replica.clone());

    let batch = OBBject::new(
        vec![Row {
            symbol: "AAPL".to_string(),
        }],
        "test",
        "rows",
    );

    let receipt = router.write_batch(&batch).await?;

    // One row x two sinks => two rows written across the fan-out.
    assert_eq!(receipt.rows_written, 2);
    assert_eq!(primary.writes()?, 1);
    assert_eq!(replica.writes()?, 1);
    println!(
        "fan-out ok: {} sinks, summed rows_written = {}",
        router.sink_count(),
        receipt.rows_written
    );
    Ok(())
}
