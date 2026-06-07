//! Offline `LocalBlobEngine` round-trip: write an object to a temp directory and
//! read it back. No network, no docker.
//!
//! Run with: `cargo run -p tdw-storage-fs --example basic`

use bytes::Bytes;
use tdw_core::BlobEngine;
use tdw_storage_fs::LocalBlobEngine;

#[tokio::main]
async fn main() -> tdw_core::Result<()> {
    // Root the engine at a unique temp directory so the example is self-contained.
    let root = std::env::temp_dir().join(format!("tdw-storage-fs-example-{}", std::process::id()));
    let engine = LocalBlobEngine::new(&root);

    let key = "raw/ohlcv.bin";
    let payload = Bytes::from_static(b"open,high,low,close\n1,2,0,1\n");

    // Store, then retrieve.
    engine.put_object(key, payload.clone(), "text/csv").await?;
    let fetched = engine.get_object(key).await?;

    assert_eq!(fetched, payload);
    println!(
        "round-trip ok: wrote {} bytes to {}/{key} and read them back",
        fetched.len(),
        root.display()
    );

    // Tidy up the temp directory (best effort).
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}
