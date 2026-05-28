#![forbid(unsafe_code)]

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
