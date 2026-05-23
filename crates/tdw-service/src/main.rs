#![forbid(unsafe_code)]

use tdw_test_utils::smoke::{SmokeReport, allocate_storage_root, run_end_to_end_smoke};

#[tokio::main]
async fn main() {
    let symbol = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "AAPL".to_string());
    let root = allocate_storage_root("tdw-service");

    let report: SmokeReport = match run_end_to_end_smoke(&symbol, root.clone()).await {
        Ok(report) => report,
        Err(error) => {
            eprintln!("tdw-service smoke error: {error}");
            std::process::exit(1);
        }
    };

    match serde_json::to_string_pretty(&report) {
        Ok(json) => println!("{json}"),
        Err(error) => {
            eprintln!("tdw-service serialize error: {error}");
            std::process::exit(1);
        }
    }

    let _ = std::fs::remove_dir_all(&root);
}
