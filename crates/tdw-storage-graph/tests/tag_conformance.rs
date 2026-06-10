//! Cross-backend behavioral conformance for the tag engine (knowledge-system
//! A5): ONE suite of assertions runs verbatim against the in-process
//! [`InMemoryTagEngine`] and the graph-backed [`GraphTagEngine`] so the two
//! backends cannot silently drift — and BOTH legs run offline (the graph leg
//! sits on the in-memory graph engine), so the parity proof needs no server.
//! The Bolt-backed composition reuses these exact semantics through the same
//! `GraphTagEngine`, inheriting the A4 graph conformance guarantees.

#![forbid(unsafe_code)]

use tdw_storage_graph::{GraphTagEngine, InMemoryGraphEngine};
use tdw_tags::{InMemoryTagEngine, TagAssignment, TagDefinition, TagEngine, TagError};

fn definition(tag_id: &str, parent: Option<&str>) -> TagDefinition {
    TagDefinition {
        tag_id: tag_id.to_string(),
        parent: parent.map(ToString::to_string),
        ttl_days: None,
    }
}

fn assignment(entity: &str, tag: &str, assigned: &str, expires: Option<&str>) -> TagAssignment {
    TagAssignment {
        entity_id: entity.to_string(),
        tag_id: tag.to_string(),
        assigned_at: assigned.to_string(),
        expires_at: expires.map(ToString::to_string),
        provenance: "conformance".to_string(),
    }
}

/// The behavioral contract, asserted identically for every backend.
// The suite is intentionally one function: splitting it would let backends
// run different subsets and break the "identical assertions" parity guarantee.
#[allow(clippy::too_many_lines)]
async fn conformance_suite(engine: &dyn TagEngine) {
    // 1. Taxonomy: parents must exist; cycles are rejected; re-parenting works.
    assert!(matches!(
        engine
            .define(definition("style:momentum", Some("asset:equity")))
            .await,
        Err(TagError::UnknownParent(_))
    ));
    for (tag, parent) in [
        ("asset:any", None),
        ("asset:equity", Some("asset:any")),
        ("asset:crypto", Some("asset:any")),
        ("asset:equity-us", Some("asset:equity")),
    ] {
        engine
            .define(definition(tag, parent))
            .await
            .unwrap_or_else(|error| panic!("{tag} should define: {error}"));
    }
    // A cycle through the chain (re-pointing the root at its grandchild).
    assert!(matches!(
        engine
            .define(definition("asset:any", Some("asset:equity-us")))
            .await,
        Err(TagError::Cycle(_))
    ));
    // The rejected redefinition must not have damaged the taxonomy.
    assert_eq!(
        engine
            .descendants("asset:any")
            .await
            .unwrap_or_else(|error| panic!("descendants should succeed: {error}")),
        vec![
            "asset:crypto".to_string(),
            "asset:equity".to_string(),
            "asset:equity-us".to_string(),
        ]
    );
    // Re-parenting (legal): move equity-us under asset:any directly, then back.
    engine
        .define(definition("asset:equity-us", Some("asset:any")))
        .await
        .unwrap_or_else(|error| panic!("re-parent should succeed: {error}"));
    assert!(
        engine
            .descendants("asset:equity")
            .await
            .unwrap_or_else(|error| panic!("descendants should succeed: {error}"))
            .is_empty(),
        "after re-parenting, equity has no descendants"
    );
    engine
        .define(definition("asset:equity-us", Some("asset:equity")))
        .await
        .unwrap_or_else(|error| panic!("re-parent back should succeed: {error}"));

    // 2. Validation shape: bad ids, zero TTL, inverted windows.
    assert!(engine.define(definition("notacolon", None)).await.is_err());
    assert!(
        engine
            .define(TagDefinition {
                tag_id: "ttl:zero".to_string(),
                parent: None,
                ttl_days: Some(0),
            })
            .await
            .is_err()
    );
    assert!(matches!(
        engine
            .assign(assignment(
                "instrument:AAPL",
                "asset:missing",
                "2026-01-01",
                None
            ))
            .await,
        Err(TagError::UnknownTag(_))
    ));
    assert!(
        engine
            .assign(assignment(
                "instrument:AAPL",
                "asset:equity",
                "2026-02-01",
                Some("2026-01-01"),
            ))
            .await
            .is_err(),
        "inverted window must be rejected"
    );

    // 3. Temporal windows: half-open [assigned_at, expires_at), dedup across
    //    overlapping windows, expiry exclusivity.
    engine
        .assign(assignment(
            "instrument:AAPL",
            "asset:equity",
            "2026-01-01",
            None,
        ))
        .await
        .unwrap_or_else(|error| panic!("assign should succeed: {error}"));
    engine
        .assign(assignment(
            "instrument:MSFT",
            "asset:equity",
            "2026-01-01",
            Some("2026-02-01"),
        ))
        .await
        .unwrap_or_else(|error| panic!("assign should succeed: {error}"));
    // Overlapping second window for AAPL — active_tags must dedup.
    engine
        .assign(assignment(
            "instrument:AAPL",
            "asset:equity",
            "2026-03-01",
            None,
        ))
        .await
        .unwrap_or_else(|error| panic!("assign should succeed: {error}"));

    assert_eq!(
        engine
            .active_tags("instrument:AAPL", "2026-03-15")
            .await
            .unwrap_or_else(|error| panic!("active_tags should succeed: {error}")),
        vec!["asset:equity".to_string()],
        "overlapping windows dedup to one tag"
    );
    assert_eq!(
        engine
            .active_tags("instrument:MSFT", "2026-01-15")
            .await
            .unwrap_or_else(|error| panic!("active_tags should succeed: {error}")),
        vec!["asset:equity".to_string()]
    );
    assert!(
        engine
            .active_tags("instrument:MSFT", "2026-02-01")
            .await
            .unwrap_or_else(|error| panic!("active_tags should succeed: {error}"))
            .is_empty(),
        "expiry day is exclusive"
    );
    assert!(
        engine
            .active_tags("instrument:AAPL", "2025-12-31")
            .await
            .unwrap_or_else(|error| panic!("active_tags should succeed: {error}"))
            .is_empty(),
        "before assignment nothing is active"
    );

    // 4. entities_with_tag: window-filtered, deduplicated, sorted.
    assert_eq!(
        engine
            .entities_with_tag("asset:equity", "2026-01-15")
            .await
            .unwrap_or_else(|error| panic!("entities should succeed: {error}")),
        vec!["instrument:AAPL".to_string(), "instrument:MSFT".to_string()]
    );
    assert_eq!(
        engine
            .entities_with_tag("asset:equity", "2026-06-01")
            .await
            .unwrap_or_else(|error| panic!("entities should succeed: {error}")),
        vec!["instrument:AAPL".to_string()],
        "expired MSFT drops; duplicate AAPL windows dedup"
    );
    assert!(
        engine
            .entities_with_tag("asset:crypto", "2026-01-15")
            .await
            .unwrap_or_else(|error| panic!("entities should succeed: {error}"))
            .is_empty()
    );

    // 5. Descendants: transitive, sorted, leaf/unknown-safe.
    assert_eq!(
        engine
            .descendants("asset:equity")
            .await
            .unwrap_or_else(|error| panic!("descendants should succeed: {error}")),
        vec!["asset:equity-us".to_string()]
    );
    assert!(
        engine
            .descendants("asset:equity-us")
            .await
            .unwrap_or_else(|error| panic!("descendants should succeed: {error}"))
            .is_empty()
    );
    assert!(
        engine
            .descendants("asset:unknown")
            .await
            .unwrap_or_else(|error| panic!("descendants should succeed: {error}"))
            .is_empty()
    );
}

/// Taxonomies deeper than the graph layer's traversal hop cap must still
/// return ALL transitive descendants on every backend (regression for the
/// silent `MAX_HOPS` truncation the A5 review caught).
async fn deep_taxonomy_suite(engine: &dyn TagEngine) {
    let mut parent: Option<String> = None;
    let mut expected = Vec::new();
    for level in 0..12_u8 {
        let tag = format!("deep:level-{level:02}");
        engine
            .define(definition(&tag, parent.as_deref()))
            .await
            .unwrap_or_else(|error| panic!("{tag} should define: {error}"));
        if level > 0 {
            expected.push(tag.clone());
        }
        parent = Some(tag);
    }
    expected.sort();
    assert_eq!(
        engine
            .descendants("deep:level-00")
            .await
            .unwrap_or_else(|error| panic!("descendants should succeed: {error}")),
        expected,
        "all 11 transitive descendants, beyond the traversal hop cap"
    );
}

#[tokio::test]
async fn in_memory_tag_engine_meets_the_contract() {
    conformance_suite(&InMemoryTagEngine::default()).await;
}

#[tokio::test]
async fn graph_backed_tag_engine_meets_the_contract() {
    conformance_suite(&GraphTagEngine::new(InMemoryGraphEngine::default())).await;
}

#[tokio::test]
async fn deep_taxonomies_are_complete_on_both_backends() {
    deep_taxonomy_suite(&InMemoryTagEngine::default()).await;
    deep_taxonomy_suite(&GraphTagEngine::new(InMemoryGraphEngine::default())).await;
}
