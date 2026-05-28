#![forbid(unsafe_code)]

#[cfg(feature = "postgres")]
pub mod pg_snapshot;

#[cfg(feature = "postgres")]
pub use pg_snapshot::PgSnapshotStore;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub table: String,
    pub version: u64,
    pub created_at: String,
    pub row_ids: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SnapshotStore {
    snapshots: Vec<Snapshot>,
}

impl SnapshotStore {
    pub fn commit(
        &mut self,
        table: impl Into<String>,
        created_at: impl Into<String>,
        row_ids: Vec<String>,
    ) -> Snapshot {
        let table = table.into();
        let version = self
            .snapshots
            .iter()
            .filter(|snapshot| snapshot.table == table)
            .map(|snapshot| snapshot.version)
            .max()
            .unwrap_or(0)
            + 1;
        let snapshot = Snapshot {
            table,
            version,
            created_at: created_at.into(),
            row_ids,
        };
        self.snapshots.push(snapshot.clone());
        snapshot
    }

    #[must_use]
    pub fn as_of_version(&self, table: &str, version: u64) -> Option<&Snapshot> {
        self.snapshots
            .iter()
            .find(|snapshot| snapshot.table == table && snapshot.version == version)
    }

    #[must_use]
    pub fn latest(&self, table: &str) -> Option<&Snapshot> {
        self.snapshots
            .iter()
            .filter(|snapshot| snapshot.table == table)
            .max_by_key(|snapshot| snapshot.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commits_versions_and_supports_time_travel_lookup() {
        let mut store = SnapshotStore::default();
        store.commit(
            "raw.market_data_bar",
            "2026-05-21T00:00:00Z",
            vec!["a".into()],
        );
        store.commit(
            "raw.market_data_bar",
            "2026-05-21T01:00:00Z",
            vec!["a".into(), "b".into()],
        );

        assert_eq!(
            store
                .as_of_version("raw.market_data_bar", 1)
                .map(|snapshot| snapshot.row_ids.clone()),
            Some(vec!["a".to_string()])
        );
        assert_eq!(
            store
                .latest("raw.market_data_bar")
                .map(|snapshot| snapshot.version),
            Some(2)
        );
    }
}
