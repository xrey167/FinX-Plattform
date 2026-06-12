//! Integration test for `tdw kg demo` (knowledge-system-2 K-X2).
//!
//! Runs the full demo path end-to-end against the checked-in fixtures and
//! asserts each step's key outputs via the `run_smoke()` contract:
//!
//! - v1 ingest: 8 fixture documents land
//! - derived supply_chain_peer edge present (DeriveEdge rule fired)
//! - why-chain: support non-empty (rule step traceable)
//! - diff: manifest delta non-empty between v1 and v2 snapshots
//! - status: document count > 0

#![forbid(unsafe_code)]

use tdw_cli::demo::run_smoke;

#[tokio::test]
async fn kg_demo_full_e2e_smoke() {
    run_smoke()
        .await
        .unwrap_or_else(|e| panic!("tdw kg demo smoke failed: {e}"));
}
