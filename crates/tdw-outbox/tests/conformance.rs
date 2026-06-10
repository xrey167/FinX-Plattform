//! Cross-backend behavioral conformance for the outbox (go-live P2.6, slice 2).
//!
//! ONE suite of assertions runs verbatim against the in-memory outbox and the
//! Postgres store so the backends cannot silently drift: append sequencing,
//! `pending_after` ordering/filtering, and the `Pending -> Dispatched`
//! transition guard behave identically wherever the outbox lives.
//!
//! All assertions are RELATIVE (strictly-increasing sequences, addressing by
//! returned sequence) so the suite holds on a shared Postgres table where
//! absolute sequence numbers are table-global.
//!
//! The Postgres leg compiles with `--features postgres` and self-skips unless
//! `TDW_POSTGRES_TEST_URL` is set, mirroring `tests/pg_outbox.rs`.

#![forbid(unsafe_code)]

use serde_json::Value;
use tdw_event::{EventEnvelope, sample_event};
use tdw_outbox::{InMemoryOutbox, OutboxRecord, OutboxStatus};

/// A sequence far beyond anything a test run appends, for the unknown-sequence
/// probe. (Postgres bigserial in any realistic test database stays far below.)
const UNKNOWN_SEQUENCE: u64 = 9_000_000_000_000_000;

enum AnyOutbox {
    Mem(InMemoryOutbox),
    #[cfg(feature = "postgres")]
    Pg(tdw_outbox::PgOutboxStore),
}

impl AnyOutbox {
    async fn append(&mut self, envelope: EventEnvelope<Value>) -> u64 {
        match self {
            Self::Mem(outbox) => outbox.append(envelope),
            #[cfg(feature = "postgres")]
            Self::Pg(store) => store
                .append(envelope)
                .await
                .unwrap_or_else(|error| panic!("append must succeed: {error}")),
        }
    }

    async fn mark_dispatched(&mut self, sequence: u64) -> bool {
        match self {
            Self::Mem(outbox) => outbox.mark_dispatched(sequence),
            #[cfg(feature = "postgres")]
            Self::Pg(store) => store
                .mark_dispatched(sequence)
                .await
                .unwrap_or_else(|error| panic!("mark_dispatched must succeed: {error}")),
        }
    }

    async fn pending_after(&self, sequence: u64) -> Vec<OutboxRecord> {
        match self {
            Self::Mem(outbox) => outbox.pending_after(sequence),
            #[cfg(feature = "postgres")]
            Self::Pg(store) => store
                .pending_after(sequence)
                .await
                .unwrap_or_else(|error| panic!("pending_after must succeed: {error}")),
        }
    }
}

/// The behavioral contract, asserted identically for every backend.
async fn conformance_suite(outbox: &mut AnyOutbox) {
    // Baseline sentinel: everything this run asserts is scoped strictly after
    // it, so the suite holds even on a Postgres table shared between runs.
    let s0 = outbox.append(sample_event("conf-sentinel")).await;
    outbox.mark_dispatched(s0).await;

    // 1. Appends hand out strictly increasing sequences.
    let s1 = outbox.append(sample_event("conf-a")).await;
    let s2 = outbox.append(sample_event("conf-b")).await;
    let s3 = outbox.append(sample_event("conf-c")).await;
    assert!(
        s0 < s1 && s1 < s2 && s2 < s3,
        "sequences must be strictly increasing: {s0} {s1} {s2} {s3}"
    );

    // 2. pending_after returns exactly the pending records, ascending.
    let pending = outbox.pending_after(s0).await;
    let sequences: Vec<u64> = pending.iter().map(|record| record.sequence).collect();
    assert_eq!(sequences, vec![s1, s2, s3], "ascending pending order");
    assert!(
        pending
            .iter()
            .all(|record| record.status == OutboxStatus::Pending),
        "pending_after must only surface Pending records"
    );

    // 3. The dispatch transition guard: true exactly once, false on repeat,
    //    false for a sequence that does not exist.
    assert!(
        outbox.mark_dispatched(s2).await,
        "first dispatch transitions"
    );
    assert!(
        !outbox.mark_dispatched(s2).await,
        "second dispatch of the same sequence must report false"
    );
    assert!(
        !outbox.mark_dispatched(UNKNOWN_SEQUENCE).await,
        "unknown sequence must report false"
    );

    // 4. Dispatched records vanish from pending_after; ordering is preserved.
    let remaining: Vec<u64> = outbox
        .pending_after(s0)
        .await
        .iter()
        .map(|record| record.sequence)
        .collect();
    assert_eq!(remaining, vec![s1, s3], "dispatched s2 is filtered out");

    // 5. Strictly-greater filtering: pending_after(s1) excludes s1 itself.
    let after_s1: Vec<u64> = outbox
        .pending_after(s1)
        .await
        .iter()
        .map(|record| record.sequence)
        .collect();
    assert_eq!(after_s1, vec![s3], "filter is strictly greater-than");

    // 6. Draining the rest leaves nothing pending after the sentinel.
    assert!(outbox.mark_dispatched(s1).await);
    assert!(outbox.mark_dispatched(s3).await);
    assert!(
        outbox.pending_after(s0).await.is_empty(),
        "fully dispatched outbox has nothing pending"
    );
}

#[tokio::test]
async fn in_memory_backend_conforms() {
    let mut outbox = AnyOutbox::Mem(InMemoryOutbox::default());
    conformance_suite(&mut outbox).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_backend_conforms() {
    use tdw_core::RelationalEngine;
    use tdw_storage_postgres::PgEngine;

    let Some(url) = std::env::var("TDW_POSTGRES_TEST_URL").ok() else {
        eprintln!("TDW_POSTGRES_TEST_URL not set; skipping postgres conformance leg");
        return;
    };
    let engine = PgEngine::connect(&url)
        .await
        .unwrap_or_else(|error| panic!("postgres connect: {error}"));

    // Hermetic per-run table, mirroring tests/pg_outbox.rs.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let table = format!("tdw_outbox_conf_{}_{}", std::process::id(), nanos);
    let store = tdw_outbox::PgOutboxStore::new(engine.clone())
        .with_table(&table)
        .expect("valid outbox table identifier");
    store
        .ensure_schema()
        .await
        .unwrap_or_else(|error| panic!("ensure_schema: {error}"));

    let mut outbox = AnyOutbox::Pg(store);
    conformance_suite(&mut outbox).await;

    let drop_sql = format!("DROP TABLE IF EXISTS {table}");
    engine
        .execute(&drop_sql, serde_json::Value::Null)
        .await
        .unwrap_or_else(|error| panic!("cleanup drop: {error}"));
}
