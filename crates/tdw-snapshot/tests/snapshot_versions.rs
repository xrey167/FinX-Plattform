#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used)]

// Integration coverage for tdw-snapshot: per-table version monotonicity,
// time-travel lookup, latest, multi-table isolation.
// Authored offline. Verify with `cargo test --package tdw-snapshot`.

use tdw_snapshot::SnapshotStore;

#[test]
fn first_commit_for_a_table_starts_at_version_one() {
    let mut store = SnapshotStore::default();
    let snap = store.commit("t", "2026-05-22T00:00:00Z", vec!["a".into()]);
    assert_eq!(snap.version, 1);
    assert_eq!(snap.table, "t");
    assert_eq!(snap.row_ids, vec!["a".to_string()]);
}

#[test]
fn versions_are_monotonic_per_table_only() {
    let mut store = SnapshotStore::default();
    let t1_v1 = store.commit("t1", "ts1", vec![]);
    let t1_v2 = store.commit("t1", "ts2", vec![]);
    let t2_v1 = store.commit("t2", "ts3", vec![]);
    let t2_v2 = store.commit("t2", "ts4", vec![]);
    assert_eq!(t1_v1.version, 1);
    assert_eq!(t1_v2.version, 2);
    assert_eq!(t2_v1.version, 1, "different table restarts at 1");
    assert_eq!(t2_v2.version, 2);
}

#[test]
fn as_of_version_returns_none_for_unknown_table() {
    let store = SnapshotStore::default();
    assert!(store.as_of_version("nope", 1).is_none());
}

#[test]
fn as_of_version_returns_none_for_unknown_version() {
    let mut store = SnapshotStore::default();
    store.commit("t", "ts", vec![]);
    assert!(store.as_of_version("t", 99).is_none());
}

#[test]
fn as_of_version_returns_exact_snapshot() {
    let mut store = SnapshotStore::default();
    store.commit("t", "ts1", vec!["a".into()]);
    store.commit("t", "ts2", vec!["a".into(), "b".into()]);
    let snap = store.as_of_version("t", 1).expect("v1 exists");
    assert_eq!(snap.row_ids, vec!["a".to_string()]);
    assert_eq!(snap.created_at, "ts1");
}

#[test]
fn latest_returns_none_for_unknown_table() {
    let store = SnapshotStore::default();
    assert!(store.latest("nope").is_none());
}

#[test]
fn latest_returns_max_version() {
    let mut store = SnapshotStore::default();
    store.commit("t", "ts1", vec!["a".into()]);
    store.commit("t", "ts2", vec!["a".into(), "b".into()]);
    store.commit("t", "ts3", vec!["a".into(), "b".into(), "c".into()]);
    let snap = store.latest("t").expect("latest exists");
    assert_eq!(snap.version, 3);
    assert_eq!(snap.row_ids.len(), 3);
}

#[test]
fn latest_for_table_with_one_snapshot_returns_that_snapshot() {
    let mut store = SnapshotStore::default();
    store.commit("t", "ts1", vec![]);
    assert_eq!(store.latest("t").map(|s| s.version), Some(1));
}

#[test]
fn snapshot_round_trips_via_serde() {
    let mut store = SnapshotStore::default();
    let snap = store.commit("t", "ts", vec!["a".into(), "b".into()]);
    let json = serde_json::to_string(&snap).unwrap_or_else(|e| panic!("serialize: {e}"));
    let decoded: tdw_snapshot::Snapshot =
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize: {e}"));
    assert_eq!(decoded, snap);
}
