//! Integration test for `tdw kg demo` (knowledge-system-2 K-X2).
//!
//! Runs the full demo path end-to-end against the checked-in fixtures and
//! asserts each step's key outputs:
//!
//! - v1 ingest: all 8 fixture documents land
//! - derived edge present in the derivation index (DeriveEdge rule fired)
//! - why-chain has at least one support (rule step traceable)
//! - diff between v1 and v2 snapshots is non-empty
//! - status: document count > 0 after ingest

#![forbid(unsafe_code)]

use tdw_cli::demo::run_smoke;

#[tokio::test]
async fn kg_demo_full_e2e_smoke() {
    run_smoke()
        .await
        .unwrap_or_else(|e| panic!("tdw kg demo smoke failed: {e}"));
}
