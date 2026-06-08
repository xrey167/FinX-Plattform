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
    #[error("invalid rule: {0}")]
    InvalidRule(&'static str),
    #[error("tag assignment failed: {0}")]
    Tag(String),
    #[error("rule recursion exceeded")]
    Recursion,
}

#[derive(Clone, Debug, Default)]
pub struct RuleEngine {
    rules: Vec<TagRule>,
    version: u64,
}

impl RuleEngine {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn hot_reload(&mut self, rules: Vec<TagRule>) -> Result<(), RuleError> {
        for rule in &rules {
            validate_rule(rule)?;
        }
        self.rules = rules;
        self.version += 1;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
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
                store
                    .assign(assignment.clone())
                    .map_err(|error| RuleError::Tag(error.to_string()))?;
                assignments.push(assignment);
            }
        }
        Ok(assignments)
    }

    #[must_use]
    pub const fn version(&self) -> u64 {
        self.version
    }
}

fn validate_rule(rule: &TagRule) -> Result<(), RuleError> {
    if !is_identifier(&rule.rule_id) {
        return Err(RuleError::InvalidRule("rule_id"));
    }
    if !is_tag_id(&rule.tag_id) {
        return Err(RuleError::InvalidRule("tag_id"));
    }
    match &rule.predicate {
        RulePredicate::SqlContains { sql, needle } => {
            if sql.contains(';')
                || sql.contains("--")
                || sql.to_ascii_lowercase().contains(" drop ")
            {
                return Err(RuleError::UnsafeSql);
            }
            if needle.trim().is_empty() {
                return Err(RuleError::InvalidRule("needle"));
            }
        }
        RulePredicate::JsonPathEquals { path, value } => {
            if !path.starts_with("$.") || path.contains("..") || path.chars().any(char::is_control)
            {
                return Err(RuleError::InvalidRule("json_path"));
            }
            if value.trim().is_empty() {
                return Err(RuleError::InvalidRule("value"));
            }
        }
        RulePredicate::LabelContains { label } => {
            if label.trim().is_empty() || label.chars().any(char::is_control) {
                return Err(RuleError::InvalidRule("label"));
            }
        }
    }
    Ok(())
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

fn is_tag_id(value: &str) -> bool {
    !value.is_empty()
        && value.contains(':')
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ':' | '_' | '-')
        })
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

    #[test]
    fn rejects_invalid_rules_and_unknown_tag_assignments() {
        let mut engine = RuleEngine::default();
        assert_eq!(
            engine.hot_reload(vec![TagRule {
                rule_id: "../bad".to_string(),
                tag_id: "asset:equity".to_string(),
                predicate: RulePredicate::LabelContains {
                    label: "AAPL".to_string(),
                },
            }]),
            Err(RuleError::InvalidRule("rule_id"))
        );
        engine
            .hot_reload(vec![TagRule {
                rule_id: "missing-tag".to_string(),
                tag_id: "asset:missing".to_string(),
                predicate: RulePredicate::LabelContains {
                    label: "AAPL".to_string(),
                },
            }])
            .unwrap_or_else(|error| panic!("rule should reload: {error}"));

        let mut tags = TagStore::default();
        assert!(matches!(
            engine.apply("instrument:AAPL", "AAPL", &mut tags),
            Err(RuleError::Tag(_))
        ));
    }

    #[test]
    fn json_path_equals_predicate_validates_and_matches_on_exact_label() {
        let mut engine = RuleEngine::default();

        // Validation: path must start with "$."
        assert_eq!(
            engine.hot_reload(vec![TagRule {
                rule_id: "bad-path".to_string(),
                tag_id: "asset:equity".to_string(),
                predicate: RulePredicate::JsonPathEquals {
                    path: "asset".to_string(),
                    value: "equity".to_string(),
                },
            }]),
            Err(RuleError::InvalidRule("json_path"))
        );
        // Validation: value must be non-empty.
        assert_eq!(
            engine.hot_reload(vec![TagRule {
                rule_id: "empty-value".to_string(),
                tag_id: "asset:equity".to_string(),
                predicate: RulePredicate::JsonPathEquals {
                    path: "$.asset".to_string(),
                    value: "   ".to_string(),
                },
            }]),
            Err(RuleError::InvalidRule("value"))
        );

        // Apply: JsonPathEquals matches only when the whole label equals `value`.
        let mut tags = TagStore::default();
        tags.define(TagDefinition {
            tag_id: "asset:equity".to_string(),
            parent: None,
            ttl_days: None,
        })
        .unwrap_or_else(|error| panic!("tag should define: {error}"));
        engine
            .hot_reload(vec![TagRule {
                rule_id: "equity-exact".to_string(),
                tag_id: "asset:equity".to_string(),
                predicate: RulePredicate::JsonPathEquals {
                    path: "$.asset_class".to_string(),
                    value: "equity".to_string(),
                },
            }])
            .unwrap_or_else(|error| panic!("rule should reload: {error}"));

        let matched = engine
            .apply("instrument:1", "equity", &mut tags)
            .unwrap_or_else(|error| panic!("apply should succeed: {error}"));
        assert_eq!(matched.len(), 1, "exact label equals value -> assigned");

        let unmatched = engine
            .apply("instrument:2", "equity-ish", &mut tags)
            .unwrap_or_else(|error| panic!("apply should succeed: {error}"));
        assert!(
            unmatched.is_empty(),
            "superstring must not match (equality, not contains)"
        );
    }
}
