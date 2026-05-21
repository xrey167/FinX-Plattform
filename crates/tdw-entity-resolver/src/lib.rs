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

pub fn resolve_symbol(symbol: &str, entities: &[Entity]) -> Vec<ResolveCandidate> {
    let normalized = symbol.to_ascii_uppercase();
    entities
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
        .collect()
}

pub fn manual_merge_decision(source: &str, target: &str, approved: bool) -> MergeDecision {
    MergeDecision {
        source: source.to_string(),
        target: target.to_string(),
        approved,
        audited: true,
    }
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
}
