//! Offline `InMemoryS3BlobEngine` round-trip: put an object and get it back.
//! No network, no docker — the default in-memory engine is always available.
//!
//! Run with: `cargo run -p tdw-storage-s3 --example basic`

use bytes::Bytes;
use tdw_core::BlobEngine;
use tdw_storage_s3::InMemoryS3BlobEngine;

#[tokio::main]
async fn main() -> tdw_core::Result<()> {
    let engine = InMemoryS3BlobEngine::default();

    let key = "raw/ohlcv.parquet";
    let payload = Bytes::from_static(b"<parquet bytes>");

    engine
        .put_object(key, payload.clone(), "application/vnd.apache.parquet")
        .await?;
    let fetched = engine.get_object(key).await?;

    assert_eq!(fetched, payload);
    println!(
        "round-trip ok: stored {key} ({} bytes), object_count = {}",
        fetched.len(),
        engine.object_count()?
    );
    Ok(())
}
