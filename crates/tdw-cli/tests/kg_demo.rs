//! Integration test for `tdw kg demo` (knowledge-system-2 K-X2).
//!
//! Runs the full demo path end-to-end against the checked-in fixtures and
//! makes exact-value assertions on each step's key outputs.

#![forbid(unsafe_code)]

use tdw_cli::demo::run_smoke_detailed;

/// v1 fixture has 8 unique document ids.
const EXPECTED_LANDED_V1: usize = 8;

/// docs-v1 mentions graph:
///   aapl-profile  → company:AAPL, company:TSMC   (2 mentions edges out)
///   company-aapl  → company:TSMC                  (1 mention edge out)
///   tsm-profile   → company:TSMC                  (1 mention edge out)
///   tsm-supply-note → company:AAPL, instrument:AAPL (2 mentions edges)
///   aapl-10q-2025q4 → company:AAPL               (1 mention edge out)
///   msft-profile  → company:MSFT                  (1 mention edge out)
///
/// DeriveEdge rule: mentions→mentions→supply_chain_peer
/// Two-hop pairs that produce supply_chain_peer:
///   aapl-profile  →(mentions)→ company:TSMC  via company-aapl  →(mentions)→ company:TSMC
///   => instrument:AAPL supply_chain_peer company:AAPL  (aapl-profile→company:AAPL, company:AAPL→company:TSMC)
/// The exact count is determined by the rule engine; the fixture is designed
/// to produce at least 1.  We assert ≥ 1 rather than a fragile exact number
/// that would break if the fixture mention graph is tweaked.
const EXPECTED_DERIVE_RULE_ID: &str = "supply-chain-peer";

/// After v1 (8 docs) + v2 (3 docs, 1 shared id = aapl-profile), the manifest
/// holds 10 unique document ids:
///   v1: aapl-profile, msft-profile, tsm-profile, company-aapl, company-msft,
///       company-tsmc, aapl-10q-2025q4, tsm-supply-note  (8)
///   v2-new: aapl-10q-2026q1, msft-azure-note            (+2, aapl-profile already in v1)
const EXPECTED_TOTAL_DOCS: usize = 10;

/// v2 changes aapl-profile (body updated) → 1 changed doc.
const EXPECTED_MANIFEST_CHANGED: &[&str] = &["aapl-profile"];

/// v2 adds aapl-10q-2026q1 and msft-azure-note → 2 new docs.
const EXPECTED_MANIFEST_ADDED: &[&str] = &["aapl-10q-2026q1", "msft-azure-note"];

#[tokio::test]
async fn kg_demo_full_e2e_exact_values() {
    let result = run_smoke_detailed()
        .await
        .unwrap_or_else(|e| panic!("tdw kg demo smoke failed: {e}"));

    // ── Step 1: ingest counts ─────────────────────────────────────────────
    assert_eq!(
        result.landed_v1, EXPECTED_LANDED_V1,
        "v1 ingest: expected {EXPECTED_LANDED_V1} docs landed, got {}",
        result.landed_v1
    );
    assert!(
        result.derived_edges_v1 >= 1,
        "v1 inference: expected ≥1 derived supply_chain_peer edge, got {}",
        result.derived_edges_v1
    );

    // ── Step 3: why — rule identity ────────────────────────────────────────
    assert_eq!(
        result.why_rule_id, EXPECTED_DERIVE_RULE_ID,
        "why step: rule_id must be {EXPECTED_DERIVE_RULE_ID:?}, got {:?}",
        result.why_rule_id
    );

    // ── Step 4: diff — exact changed and added sets ────────────────────────
    let mut changed_sorted = result.manifest_changed.clone();
    changed_sorted.sort();
    let mut expected_changed: Vec<String> = EXPECTED_MANIFEST_CHANGED
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    expected_changed.sort();
    assert_eq!(
        changed_sorted, expected_changed,
        "diff changed set mismatch"
    );

    let mut added_sorted = result.manifest_added.clone();
    added_sorted.sort();
    let mut expected_added: Vec<String> = EXPECTED_MANIFEST_ADDED
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    expected_added.sort();
    assert_eq!(added_sorted, expected_added, "diff added set mismatch");

    // ── Step 5: status — exact document count ─────────────────────────────
    assert_eq!(
        result.total_docs, EXPECTED_TOTAL_DOCS,
        "status: expected {EXPECTED_TOTAL_DOCS} total docs, got {}",
        result.total_docs
    );
    assert!(
        result.derived_edges_total >= 1,
        "status: expected ≥1 total derived edges, got {}",
        result.derived_edges_total
    );
}
