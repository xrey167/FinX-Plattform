//! Offline `SqliteSessionStore` round-trip: persist a session + a cost-ledger
//! entry to an in-memory SQLite database and read them back. No network, no
//! docker — the SQLite store is the always-available default and auto-migrates
//! on connect.
//!
//! Run with: `cargo run -p tdw-session --example tdw-session-basic`

use tdw_protocol::SessionId;
use tdw_session::{CostLedgerEntry, SessionRecord, SessionStatus, SqliteSessionStore};

#[tokio::main]
async fn main() -> tdw_session::Result<()> {
    let store = SqliteSessionStore::connect("sqlite::memory:").await?;
    let id = SessionId::new("session-1").expect("session id");

    store
        .upsert_session(&SessionRecord {
            session_id: id.as_str().to_string(),
            status: SessionStatus::Active,
            created_at: "2026-05-22T00:00:00Z".to_string(),
            updated_at: "2026-05-22T00:00:00Z".to_string(),
        })
        .await?;

    store
        .append_cost(&CostLedgerEntry {
            session_id: id.as_str().to_string(),
            operation_id: "op-1".to_string(),
            tokens: 42,
            bytes_scanned: 2048,
            rows_read: 128,
            rows_written: 4,
            backend: "sqlite".to_string(),
        })
        .await?;

    let session = store.get_session(&id).await?.expect("session exists");
    let costs = store.cost_entries(&id).await?;

    assert_eq!(session.status, SessionStatus::Active);
    assert_eq!(costs.len(), 1);
    println!(
        "session ok: status = {:?}, cost entries = {}, first op tokens = {}",
        session.status,
        costs.len(),
        costs[0].tokens
    );
    Ok(())
}
