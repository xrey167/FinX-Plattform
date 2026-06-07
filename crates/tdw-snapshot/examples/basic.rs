//! Offline `SnapshotStore` round-trip: commit two versions of a table and do a
//! time-travel lookup. No network, no docker — the in-memory store is the
//! always-available default.
//!
//! Run with: `cargo run -p tdw-snapshot --example tdw-snapshot-basic`

use tdw_snapshot::SnapshotStore;

fn main() {
    let mut store = SnapshotStore::default();

    let v1 = store.commit(
        "raw.market_data_bar",
        "2026-05-21T00:00:00Z",
        vec!["a".to_string()],
    );
    let v2 = store.commit(
        "raw.market_data_bar",
        "2026-05-21T01:00:00Z",
        vec!["a".to_string(), "b".to_string()],
    );

    // Versions are allocated monotonically per table.
    assert_eq!(v1.version, 1);
    assert_eq!(v2.version, 2);

    // Time-travel: read the table as of version 1.
    let as_of_v1 = store
        .as_of_version("raw.market_data_bar", 1)
        .expect("version 1 exists");
    assert_eq!(as_of_v1.row_ids, vec!["a".to_string()]);

    let latest = store.latest("raw.market_data_bar").expect("latest exists");
    assert_eq!(latest.version, 2);

    println!(
        "snapshot ok: latest version = {}, rows as_of v1 = {:?}",
        latest.version, as_of_v1.row_ids
    );
}
