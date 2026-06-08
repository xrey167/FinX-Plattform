//! Offline, no-network example for `tdw-test-utils`.
//!
//! Shows the deterministic fixtures, the integration container specs (metadata
//! only — nothing is launched), and the offline end-to-end smoke that the
//! service binaries run via `--smoke`. The smoke writes into a per-process temp
//! directory and reads it straight back; no network, no Docker.
//!
//! Run with: `cargo run -p tdw-test-utils --example tdw_test_utils_basic`

use tdw_test_utils::smoke::{allocate_storage_root, run_end_to_end_smoke};
use tdw_test_utils::{containers, fixtures};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Deterministic fixtures.
    let bars = fixtures::ohlcv("AAPL");
    println!(
        "ohlcv fixture rows: {} (first close {})",
        bars.len(),
        bars[0].close
    );
    let instrument = fixtures::instrument("AAPL");
    println!("instrument: {} @ {}", instrument.symbol, instrument.venue);

    // 2. Container specs are metadata describing the integration profile.
    let pg = containers::postgres();
    let ch = containers::clickhouse();
    println!(
        "container specs: {}:{} (image {}), {}:{}",
        pg.name, pg.default_port, pg.image, ch.name, ch.default_port,
    );

    // 3. Offline end-to-end smoke: fetch -> serialize -> blob put/get roundtrip.
    let root = allocate_storage_root("tdw-test-utils-example");
    let report = run_end_to_end_smoke("AAPL", root.clone()).await?;
    println!(
        "smoke: provider={} endpoint={} rows={} roundtrip_ok={}",
        report.provider, report.endpoint, report.rows_fetched, report.roundtrip_ok,
    );

    // Tidy up the scratch directory the smoke created.
    let _ = std::fs::remove_dir_all(&root);
    Ok(())
}
