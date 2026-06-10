#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
pub mod openfigi;

use serde::{Deserialize, Serialize};
use tdw_kg::{Entity, EntityKind};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveCandidate {
    pub entity_id: String,
    pub score: u8,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeDecision {
    pub source: String,
    pub target: String,
    pub approved: bool,
    pub audited: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResolveError {
    InvalidSymbol,
    InvalidMergeEndpoint,
    InvalidIdentifier,
}

/// A standardized-identifier crosswalk row mirroring `ref.identifier_xref`
/// (FIGI/ISIN/CUSIP/SEDOL/ticker -> instrument). Held in memory so identifier
/// resolution needs no database round-trip.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentifierRecord {
    pub scheme: String,
    pub value: String,
    pub instrument_id: String,
}

#[must_use]
pub fn resolve_symbol(symbol: &str, entities: &[Entity]) -> Vec<ResolveCandidate> {
    try_resolve_symbol(symbol, entities).unwrap_or_default()
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn try_resolve_symbol(
    symbol: &str,
    entities: &[Entity],
) -> Result<Vec<ResolveCandidate>, ResolveError> {
    if !is_symbol(symbol) {
        return Err(ResolveError::InvalidSymbol);
    }
    let normalized = symbol.to_ascii_uppercase();
    Ok(entities
        .iter()
        .filter(|entity| {
            entity.kind == EntityKind::Instrument
                && (entity.label.eq_ignore_ascii_case(&normalized)
                    || entity
                        .aliases
                        .iter()
                        .any(|alias| alias.eq_ignore_ascii_case(&normalized)))
        })
        .map(|entity| ResolveCandidate {
            entity_id: entity.entity_id.clone(),
            score: 100,
            reason: "exact symbol or alias match".to_string(),
        })
        .collect())
}

/// Resolve an instrument by a standardized identifier (e.g. FIGI or ISIN)
/// against an in-memory crosswalk, mirroring the alias path of
/// [`resolve_symbol`].
///
/// The scheme match is case-insensitive; the value match is
/// exact after trimming. Returns the matching candidates (normally one).
#[must_use]
pub fn resolve_by_identifier(
    scheme: &str,
    value: &str,
    records: &[IdentifierRecord],
) -> Vec<ResolveCandidate> {
    try_resolve_by_identifier(scheme, value, records).unwrap_or_default()
}

/// # Errors
///
/// Returns [`ResolveError::InvalidIdentifier`] if the scheme or value is empty
/// or the value contains characters outside the allowed identifier set.
pub fn try_resolve_by_identifier(
    scheme: &str,
    value: &str,
    records: &[IdentifierRecord],
) -> Result<Vec<ResolveCandidate>, ResolveError> {
    if !is_identifier_scheme(scheme) || !is_identifier_value(value) {
        return Err(ResolveError::InvalidIdentifier);
    }
    let value = value.trim();
    Ok(records
        .iter()
        .filter(|record| {
            record.scheme.eq_ignore_ascii_case(scheme) && record.value.eq_ignore_ascii_case(value)
        })
        .map(|record| ResolveCandidate {
            entity_id: record.instrument_id.clone(),
            score: 100,
            reason: format!("exact {} identifier match", scheme.to_ascii_uppercase()),
        })
        .collect())
}

/// Project a crosswalk row onto the graph (knowledge-system A5).
///
/// Produces a `Symbol` node (`symbol:<scheme>:<value>`, both lowercased to
/// the canonical key) plus an `identifies` edge to the instrument. The
/// caller upserts the instrument node first (the graph engine enforces edge
/// endpoints).
///
/// # Errors
///
/// Returns [`ResolveError::InvalidIdentifier`] for an invalid scheme/value.
pub fn identifier_to_graph(
    record: &IdentifierRecord,
) -> Result<(tdw_core::GraphNode, tdw_core::GraphEdge), ResolveError> {
    if !is_identifier_scheme(&record.scheme) || !is_identifier_value(&record.value) {
        return Err(ResolveError::InvalidIdentifier);
    }
    let scheme = record.scheme.trim().to_ascii_lowercase();
    // The value is lowercased too: the in-memory resolver matches values
    // case-INSENSITIVELY, so the graph key must be canonical or the two
    // resolve paths silently diverge on case-mismatched lookups.
    let value = record.value.trim().to_ascii_lowercase();
    let node_id = format!("symbol:{scheme}:{value}");
    let node = tdw_core::GraphNode {
        id: node_id.clone(),
        kind: tdw_kg::EntityKind::Symbol,
        label: value.clone(),
        aliases: Vec::new(),
        props: serde_json::json!({ "scheme": scheme, "value": value }),
        valid_from: None,
        valid_to: None,
    };
    let edge = tdw_core::GraphEdge {
        from: node_id,
        to: record.instrument_id.clone(),
        rel: "identifies".to_string(),
        props: serde_json::Value::Null,
        provenance: tdw_core::Provenance::Ingest {
            source: "tdw-entity-resolver:crosswalk".to_string(),
        },
        valid_from: None,
        valid_to: None,
    };
    Ok((node, edge))
}

/// Resolve an instrument by standardized identifier against the GRAPH-backed
/// crosswalk — the durable form of [`resolve_by_identifier`].
///
/// Uses [`tdw_core::Error`] so storage faults stay distinguishable from a
/// clean no-match (which is an empty candidate list).
///
/// # Errors
///
/// Returns [`tdw_core::Error::InvalidQuery`] for an invalid scheme/value, or
/// the graph engine's own error on storage failure.
pub async fn resolve_by_identifier_graph(
    graph: &dyn tdw_core::GraphEngine,
    scheme: &str,
    value: &str,
) -> tdw_core::Result<Vec<ResolveCandidate>> {
    if !is_identifier_scheme(scheme) || !is_identifier_value(value) {
        return Err(tdw_core::Error::InvalidQuery(format!(
            "invalid identifier scheme/value: {scheme:?}/{value:?}"
        )));
    }
    let node_id = format!(
        "symbol:{}:{}",
        scheme.trim().to_ascii_lowercase(),
        value.trim().to_ascii_lowercase()
    );
    let filter = tdw_core::TraversalFilter {
        rels: Some(vec!["identifies".to_string()]),
        direction: tdw_core::Direction::Out,
        max_hops: 1,
        ..tdw_core::TraversalFilter::default()
    };
    let reached = graph.neighbors(&node_id, &filter).await?;
    Ok(reached
        .into_iter()
        .map(|(_, node)| ResolveCandidate {
            entity_id: node.id,
            score: 100,
            reason: format!("exact {} identifier match", scheme.to_ascii_uppercase()),
        })
        .collect())
}

#[must_use]
pub fn manual_merge_decision(source: &str, target: &str, approved: bool) -> MergeDecision {
    MergeDecision {
        source: source.to_string(),
        target: target.to_string(),
        approved,
        audited: true,
    }
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn try_manual_merge_decision(
    source: &str,
    target: &str,
    approved: bool,
) -> Result<MergeDecision, ResolveError> {
    if !is_entity_id(source) || !is_entity_id(target) || source == target {
        return Err(ResolveError::InvalidMergeEndpoint);
    }
    Ok(manual_merge_decision(source, target, approved))
}

fn is_symbol(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed == value
        && trimmed.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
}

fn is_entity_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ':' | '.' | '_' | '-')
        })
}

fn is_identifier_scheme(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

fn is_identifier_value(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_exact_symbol_and_requires_manual_merge_approval() {
        let entities = vec![Entity {
            entity_id: "instrument:AAPL".to_string(),
            kind: EntityKind::Instrument,
            label: "Apple".to_string(),
            aliases: vec!["AAPL".to_string()],
        }];
        assert_eq!(resolve_symbol("aapl", &entities)[0].score, 100);
        assert_eq!(
            manual_merge_decision("instrument:AAPL", "instrument:APPLE", true),
            MergeDecision {
                source: "instrument:AAPL".to_string(),
                target: "instrument:APPLE".to_string(),
                approved: true,
                audited: true,
            }
        );
    }

    #[test]
    fn resolves_instrument_by_standardized_identifier() {
        let records = vec![
            IdentifierRecord {
                scheme: "FIGI".to_string(),
                value: "BBG000B9XRY4".to_string(),
                instrument_id: "INST-AAPL-XNAS".to_string(),
            },
            IdentifierRecord {
                scheme: "ISIN".to_string(),
                value: "US0378331005".to_string(),
                instrument_id: "INST-AAPL-XNAS".to_string(),
            },
        ];

        let by_figi = resolve_by_identifier("figi", "BBG000B9XRY4", &records);
        assert_eq!(by_figi.len(), 1);
        assert_eq!(by_figi[0].entity_id, "INST-AAPL-XNAS");
        assert_eq!(by_figi[0].score, 100);

        let by_isin = resolve_by_identifier("ISIN", "US0378331005", &records);
        assert_eq!(by_isin[0].entity_id, "INST-AAPL-XNAS");

        assert!(resolve_by_identifier("FIGI", "NOPE00000000", &records).is_empty());
    }

    #[tokio::test]
    async fn graph_crosswalk_round_trips_identifier_resolution() {
        use tdw_storage_graph::InMemoryGraphEngine;
        let graph = InMemoryGraphEngine::default();
        // The instrument node must exist before the identifies edge lands.
        tdw_core::GraphEngine::upsert_nodes(
            &graph,
            vec![tdw_core::GraphNode {
                id: "INST-AAPL-XNAS".to_string(),
                kind: tdw_kg::EntityKind::Instrument,
                label: "Apple".to_string(),
                aliases: Vec::new(),
                props: serde_json::Value::Null,
                valid_from: None,
                valid_to: None,
            }],
        )
        .await
        .unwrap_or_else(|error| panic!("instrument should upsert: {error}"));
        let (node, edge) = identifier_to_graph(&IdentifierRecord {
            scheme: "FIGI".to_string(),
            value: "BBG000B9XRY4".to_string(),
            instrument_id: "INST-AAPL-XNAS".to_string(),
        })
        .unwrap_or_else(|error| panic!("crosswalk should convert: {error:?}"));
        assert_eq!(
            node.id, "symbol:figi:bbg000b9xry4",
            "canonical lowercase key"
        );
        tdw_core::GraphEngine::upsert_nodes(&graph, vec![node])
            .await
            .unwrap_or_else(|error| panic!("symbol should upsert: {error}"));
        tdw_core::GraphEngine::upsert_edges(&graph, vec![edge])
            .await
            .unwrap_or_else(|error| panic!("edge should upsert: {error}"));

        // Case parity with the in-memory resolver: a lowercase query finds an
        // uppercase-ingested value (and vice versa).
        let candidates = resolve_by_identifier_graph(&graph, "FIGI", "bbg000b9xry4")
            .await
            .unwrap_or_else(|error| panic!("graph resolve should succeed: {error}"));
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].entity_id, "INST-AAPL-XNAS");
        assert_eq!(candidates[0].score, 100);
        // Unknown identifier: clean no-match, not an error.
        assert!(
            resolve_by_identifier_graph(&graph, "figi", "NOPE00000000")
                .await
                .unwrap_or_else(|error| panic!("resolve should succeed: {error}"))
                .is_empty()
        );
        // Invalid input is a loud InvalidQuery.
        assert!(resolve_by_identifier_graph(&graph, "", "X").await.is_err());
    }

    #[test]
    fn identifier_resolution_rejects_unsafe_inputs() {
        assert_eq!(
            try_resolve_by_identifier("FIGI", "../../secret", &[]),
            Err(ResolveError::InvalidIdentifier)
        );
        assert_eq!(
            try_resolve_by_identifier("", "BBG000B9XRY4", &[]),
            Err(ResolveError::InvalidIdentifier)
        );
    }

    #[test]
    fn checked_resolution_rejects_unsafe_symbols_and_self_merges() {
        assert_eq!(
            try_resolve_symbol("AAPL/../../secret", &[]),
            Err(ResolveError::InvalidSymbol)
        );
        assert_eq!(
            try_manual_merge_decision("instrument:AAPL", "instrument:AAPL", true),
            Err(ResolveError::InvalidMergeEndpoint)
        );
    }
}
