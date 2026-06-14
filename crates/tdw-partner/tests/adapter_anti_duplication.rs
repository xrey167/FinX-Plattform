//! W6.1 — the anti-duplication invariant (partner-system W6.1, partner §6).
//!
//! The design's directive 1 is "shared core, thin adapter": Partner Core is
//! built ONCE, and the three surfaces (MCP, Workspace, CLI) are *thin* adapters
//! that only map `PartnerEvent`/`Principal` to their wire format. **No routing,
//! retrieval, learning, or autonomy logic lives in any adapter** — that is the
//! explicit anti-duplication invariant, and the design names the exact review
//! test (partner §6): assert the adapters contain no `tdw_endpoint_catalog`,
//! `ProposalQueue`, or `self_tune` calls — only `PartnerCore` does.
//!
//! This is a SOURCE-LEVEL review gate: it reads the adapter source files and
//! asserts none of them references the core-logic symbols. A regression — an
//! adapter that starts resolving routes, submitting proposals, or self-tuning on
//! its own instead of delegating to the shared core — fails this test loudly.

use std::path::PathBuf;

/// The core-logic symbols that must live ONLY in `tdw-partner` (the shared core),
/// never in an adapter (the partner design §6 anti-duplication invariant).
///
/// - `tdw_endpoint_catalog` — route resolution (the §1.4 resolve seam);
/// - `ProposalQueue` — the gated KG-write surface (the §1.3 write-back);
/// - `self_tune` — the self-tuning learning loop (the §3 learning wiring).
///
/// An adapter that names any of these is doing core work it should delegate.
const FORBIDDEN_CORE_SYMBOLS: [&str; 3] = ["tdw_endpoint_catalog", "ProposalQueue", "self_tune"];

/// The three thin adapters, each relative to this crate's manifest dir.
///
/// `tdw-mcp` (MCP), `tdw-service-api/agent_bridge` (Workspace), and `tdw-cli`
/// (CLI) — the surfaces §6 names. Paths are workspace-relative from
/// `crates/tdw-partner`.
const ADAPTER_FILES: [&str; 5] = [
    "../tdw-mcp/src/partner_tools.rs",
    "../tdw-mcp/src/partner_brief_tools.rs",
    "../tdw-mcp/src/partner_audit_tools.rs",
    "../tdw-service-api/src/agent_bridge.rs",
    "../tdw-cli/src/partner.rs",
];

/// Strip `//`/`//!`/`///` line comments and the doc-narrative so the scan only
/// inspects real code. The forbidden symbols are named freely in docstrings
/// (they describe what the SHARED CORE does), so a naive substring scan would
/// false-positive on prose. We drop everything from the first `//` on each line.
fn strip_line_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| line.find("//").map_or(line, |idx| &line[..idx]))
        .collect::<Vec<_>>()
        .join("\n")
}

fn adapter_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

#[test]
fn adapters_contain_no_core_logic_symbols() {
    for relative in ADAPTER_FILES {
        let path = adapter_path(relative);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read adapter {}: {error}", path.display()));
        let code = strip_line_comments(&source);
        for symbol in FORBIDDEN_CORE_SYMBOLS {
            assert!(
                !code.contains(symbol),
                "anti-duplication invariant (partner §6, W6.1): adapter {relative} must NOT \
                 reference core symbol `{symbol}` — that logic belongs in the shared PartnerCore, \
                 the adapter only maps events/principal to its wire format"
            );
        }
    }
}

#[test]
fn adapters_delegate_to_partner_core() {
    // The positive side of the invariant: each adapter DOES go through the shared
    // core (it names `tdw_partner` / `PartnerCore`), proving it is an adapter over
    // the core rather than a parallel implementation.
    for relative in ADAPTER_FILES {
        let source = std::fs::read_to_string(adapter_path(relative))
            .unwrap_or_else(|error| panic!("read adapter {relative}: {error}"));
        assert!(
            source.contains("tdw_partner") || source.contains("PartnerCore"),
            "adapter {relative} must delegate to the shared tdw_partner core"
        );
    }
}

#[test]
fn the_anti_duplication_scan_covers_every_partner_adapter() {
    // Guard the guard: every adapter source we expect to scan exists on disk, so
    // a renamed/removed adapter cannot silently drop out of the review gate.
    for relative in ADAPTER_FILES {
        let path = adapter_path(relative);
        assert!(
            path.exists(),
            "the anti-duplication scan expects adapter {} to exist",
            path.display()
        );
    }
}
