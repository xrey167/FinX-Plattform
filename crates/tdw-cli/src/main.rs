#![forbid(unsafe_code)]

use tdw_test_utils::smoke::{SmokeReport, allocate_storage_root, run_end_to_end_smoke};

#[tokio::main]
async fn main() {
    let symbol = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "AAPL".to_string());
    let root = allocate_storage_root("tdw-cli");

    let report: SmokeReport = match run_end_to_end_smoke(&symbol, root.clone()).await {
        Ok(report) => report,
        Err(error) => {
            eprintln!("tdw-cli smoke error: {error}");
            std::process::exit(1);
        }
    };

    println!(
        "tdw-cli provider={} endpoint={} symbol={} rows={} blob={} bytes={} roundtrip={}",
        report.provider,
        report.endpoint,
        report.query_symbol,
        report.rows_fetched,
        report.blob_key,
        report.blob_bytes_written,
        report.roundtrip_ok,
    );

    let _ = std::fs::remove_dir_all(&root);
}
