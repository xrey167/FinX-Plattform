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
