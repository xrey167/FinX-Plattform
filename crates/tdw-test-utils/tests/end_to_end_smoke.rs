//! G009 end-to-end functional smoke integration test.
//!
//! Drives [`tdw_test_utils::smoke::run_end_to_end_smoke`] deterministically
//! and asserts the field-by-field shape of [`SmokeReport`]. This is the
//! baseline "still works" check that subsequent production tranches must
//! keep green.

use tdw_test_utils::smoke::{allocate_storage_root, run_end_to_end_smoke};

#[tokio::test]
async fn end_to_end_smoke_drives_runtime_provider_and_storage() {
    let root = allocate_storage_root("tdw-smoke-integ");
    let report = run_end_to_end_smoke("AAPL", root.clone())
        .await
        .unwrap_or_else(|error| panic!("smoke must succeed: {error}"));

    assert_eq!(report.provider, "fileset");
    assert_eq!(report.endpoint, "equity_historical");
    assert_eq!(report.query_symbol, "AAPL");
    assert_eq!(report.rows_fetched, 2);
    assert_eq!(report.blob_key, "smoke/AAPL.json");
    assert!(report.blob_bytes_written > 0);
    assert_eq!(report.blob_bytes_written, report.blob_bytes_read);
    assert!(report.roundtrip_ok);
    assert!(
        std::path::Path::new(&report.storage_root).exists(),
        "storage root must be materialized: {}",
        report.storage_root
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn end_to_end_smoke_normalizes_query_symbol() {
    let root = allocate_storage_root("tdw-smoke-integ-norm");
    let report = run_end_to_end_smoke(" msft ", root.clone())
        .await
        .unwrap_or_else(|error| panic!("smoke must succeed: {error}"));

    assert_eq!(report.query_symbol, "MSFT");
    assert_eq!(report.blob_key, "smoke/MSFT.json");

    let _ = std::fs::remove_dir_all(&root);
}
