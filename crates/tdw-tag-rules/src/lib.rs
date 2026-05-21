#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use tdw_tags::{TagAssignment, TagStore};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RulePredicate {
    SqlContains { sql: String, needle: String },
    JsonPathEquals { path: String, value: String },
    LabelContains { label: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagRule {
    pub rule_id: String,
    pub tag_id: String,
    pub predicate: RulePredicate,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RuleError {
    #[error("rejected unsafe SQL predicate")]
    UnsafeSql,
    #[error("rule recursion exceeded")]
    Recursion,
}

#[derive(Clone, Debug, Default)]
pub struct RuleEngine {
    rules: Vec<TagRule>,
    version: u64,
}

impl RuleEngine {
    pub fn hot_reload(&mut self, rules: Vec<TagRule>) -> Result<(), RuleError> {
        for rule in &rules {
            if let RulePredicate::SqlContains { sql, .. } = &rule.predicate
                && (sql.contains(';') || sql.contains("--"))
            {
                return Err(RuleError::UnsafeSql);
            }
        }
        self.rules = rules;
        self.version += 1;
        Ok(())
    }

    pub fn apply(
        &self,
        entity_id: &str,
        label: &str,
        store: &mut TagStore,
    ) -> Result<Vec<TagAssignment>, RuleError> {
        let mut assignments = Vec::new();
        for rule in &self.rules {
            let matched = match &rule.predicate {
                RulePredicate::SqlContains { needle, .. } => label.contains(needle),
                RulePredicate::JsonPathEquals { value, .. } => label == value,
                RulePredicate::LabelContains { label: expected } => label.contains(expected),
            };
            if matched {
                let assignment = TagAssignment {
                    entity_id: entity_id.to_string(),
                    tag_id: rule.tag_id.clone(),
                    assigned_at: "2026-05-21".to_string(),
                    expires_at: None,
                    provenance: format!("rule:{}", rule.rule_id),
                };
                let _ = store.assign(assignment.clone());
                assignments.push(assignment);
            }
        }
        Ok(assignments)
    }

    pub fn version(&self) -> u64 {
        self.version
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_tags::TagDefinition;

    #[test]
    fn hot_reload_applies_rule_and_rejects_sql_injection() {
        let mut tags = TagStore::default();
        tags.define(TagDefinition {
            tag_id: "asset:equity".to_string(),
            parent: None,
            ttl_days: None,
        })
        .unwrap_or_else(|error| panic!("tag should define: {error}"));

        let mut engine = RuleEngine::default();
        engine
            .hot_reload(vec![TagRule {
                rule_id: "equity-symbol".to_string(),
                tag_id: "asset:equity".to_string(),
                predicate: RulePredicate::LabelContains {
                    label: "AAPL".to_string(),
                },
            }])
            .unwrap_or_else(|error| panic!("rule should reload: {error}"));
        let assignments = engine
            .apply("instrument:AAPL", "AAPL", &mut tags)
            .unwrap_or_else(|error| panic!("rule should apply: {error}"));

        assert_eq!(engine.version(), 1);
        assert_eq!(assignments[0].provenance, "rule:equity-symbol");
        assert_eq!(
            engine.hot_reload(vec![TagRule {
                rule_id: "bad".to_string(),
                tag_id: "asset:equity".to_string(),
                predicate: RulePredicate::SqlContains {
                    sql: "select * from tags; drop table tags".to_string(),
                    needle: "AAPL".to_string(),
                },
            }]),
            Err(RuleError::UnsafeSql)
        );
    }
}
