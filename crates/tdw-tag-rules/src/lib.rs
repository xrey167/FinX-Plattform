#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]

//! Hot-reloadable auto-tagging rules (knowledge-system B3).
//!
//! v2 upgrades the engine from label-string matching to structured predicates over an
//! [`EntityContext`] — document/provider fields, the entity's active tags (with taxonomy
//! subsumption via `TagStore::descendants`), and pre-fetched graph neighbors
//! ([`NeighborView`]) — composable with `All`/`Any`/`Not` up to depth 4. The graph
//! context is pre-fetched by the (async) caller so the engine stays sync and
//! deterministic. `now` is injected ([`RuleEngine::apply_at`]); nothing here reads a
//! clock.
//!
//! The three v1 label predicates remain as deprecated variants with unchanged serde
//! shapes, so persisted v1 rules keep deserializing.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tdw_tags::{TagAssignment, TagStore};
use thiserror::Error;

/// Maximum nesting depth for `All`/`Any`/`Not` combinators.
pub const MAX_PREDICATE_DEPTH: usize = 4;

/// A graph neighbor visible to [`Predicate::HasNeighbor`], pre-fetched by the caller
/// (the async indexer) so rule evaluation stays sync and deterministic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeighborView {
    /// The connecting relationship type.
    pub edge_type: String,
    /// The neighbor's entity id.
    pub entity_id: String,
    /// The neighbor's taxonomy kind token (lowercase), when known.
    #[serde(default)]
    pub kind: Option<String>,
    /// The neighbor's active tags at evaluation time.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Everything a rule may inspect about one entity.
#[derive(Clone, Copy, Debug)]
pub struct EntityContext<'a> {
    /// The entity being tagged.
    pub entity_id: &'a str,
    /// Human-readable label (the only input v1 rules see).
    pub label: &'a str,
    /// Structured document/provider metadata.
    pub fields: &'a Value,
    /// The entity's active tags at evaluation time.
    pub active_tags: &'a [String],
    /// Pre-fetched graph neighborhood.
    pub neighbors: &'a [NeighborView],
}

impl<'a> EntityContext<'a> {
    /// A label-only context — exactly what v1 rules evaluated against.
    #[must_use]
    pub const fn label_only(entity_id: &'a str, label: &'a str) -> Self {
        Self {
            entity_id,
            label,
            fields: &Value::Null,
            active_tags: &[],
            neighbors: &[],
        }
    }
}

/// A tagging predicate.
///
/// The first three variants are the deprecated v1 label matchers (serde shapes
/// unchanged); the rest are the v2 structured/graph-context forms.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Predicate {
    /// DEPRECATED (v1): substring match of `needle` against the label; `sql` is
    /// only validated (injection denylist), never executed.
    SqlContains { sql: String, needle: String },
    /// DEPRECATED (v1): exact label equality with `value`; `path` is only validated.
    JsonPathEquals { path: String, value: String },
    /// DEPRECATED (v1): substring match against the label.
    LabelContains { label: String },
    /// `fields[field] == value` (dotted path, string-typed).
    FieldEquals { field: String, value: String },
    /// `fields[field]` contains `needle` (dotted path, string-typed).
    FieldContains { field: String, needle: String },
    /// `fields[field]` equals any of `values`, or is an array intersecting them.
    FieldMatchesAny { field: String, values: Vec<String> },
    /// The entity actively holds `tag` — or any taxonomy descendant of it when
    /// `include_descendants` (subsumption via `TagStore::descendants`).
    HasTag {
        tag: String,
        include_descendants: bool,
    },
    /// Some pre-fetched neighbor is connected via `edge_type` and (when given)
    /// holds `target_tag` / is of `target_kind`.
    HasNeighbor {
        edge_type: String,
        #[serde(default)]
        target_tag: Option<String>,
        #[serde(default)]
        target_kind: Option<String>,
    },
    /// Every sub-predicate holds.
    All(Vec<Self>),
    /// At least one sub-predicate holds.
    Any(Vec<Self>),
    /// The sub-predicate does not hold.
    Not(Box<Self>),
}

/// v1 name for [`Predicate`], kept so existing constructors compile unchanged.
pub type RulePredicate = Predicate;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagRule {
    pub rule_id: String,
    pub tag_id: String,
    pub predicate: Predicate,
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
    /// Atomically replace the rule set: every rule validates or nothing changes;
    /// success bumps the version.
    ///
    /// # Errors
    ///
    /// Returns the first validation failure without mutating the engine.
    pub fn hot_reload(&mut self, rules: Vec<TagRule>) -> Result<(), RuleError> {
        for rule in &rules {
            validate_rule(rule)?;
        }
        self.rules = rules;
        self.version += 1;
        Ok(())
    }

    /// Evaluate every rule against `ctx` and assign matching tags effective `now`
    /// (a `YYYY-MM-DD` date — injected, never read from a clock). Assignment
    /// provenance is `rule:<rule_id>@v<version>` so derived facts are traceable
    /// to the exact rule-set version that fired.
    ///
    /// A tag the entity ALREADY holds (per `ctx.active_tags`) is not
    /// re-assigned: the store is append-only, so re-asserting on every
    /// re-evaluation would duplicate the assignment without changing what is
    /// active (knowledge-system B5 idempotency).
    ///
    /// # Errors
    ///
    /// Returns [`RuleError::Tag`] when an assignment is rejected by the store
    /// (unknown tag, invalid `now` date).
    pub fn apply_at(
        &self,
        ctx: &EntityContext<'_>,
        now: &str,
        store: &mut TagStore,
    ) -> Result<Vec<TagAssignment>, RuleError> {
        let mut assignments = Vec::new();
        for rule in &self.rules {
            if ctx.active_tags.contains(&rule.tag_id) {
                continue;
            }
            if evaluate(&rule.predicate, ctx, store) {
                let assignment = TagAssignment {
                    entity_id: ctx.entity_id.to_string(),
                    tag_id: rule.tag_id.clone(),
                    assigned_at: now.to_string(),
                    expires_at: None,
                    provenance: format!("rule:{}@v{}", rule.rule_id, self.version),
                };
                store
                    .assign(assignment.clone())
                    .map_err(|error| RuleError::Tag(error.to_string()))?;
                assignments.push(assignment);
            }
        }
        Ok(assignments)
    }

    /// Label-only convenience (the v1 surface, now with injected `now`).
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn apply(
        &self,
        entity_id: &str,
        label: &str,
        now: &str,
        store: &mut TagStore,
    ) -> Result<Vec<TagAssignment>, RuleError> {
        self.apply_at(&EntityContext::label_only(entity_id, label), now, store)
    }

    /// Returns an iterator over the tag ids assigned by the current rule set.
    ///
    /// Used by [`KnowledgeIndexer`] to pre-define rule-target tags in its
    /// internal [`TagStore`] so that [`apply_at`] can assign them without
    /// hitting [`TagError::UnknownTag`] (knowledge-system K-L1 U1 fix).
    ///
    /// [`KnowledgeIndexer`]: tdw_knowledge::KnowledgeIndexer
    /// [`apply_at`]: Self::apply_at
    /// [`TagError::UnknownTag`]: tdw_tags::TagError::UnknownTag
    pub fn tag_ids(&self) -> impl Iterator<Item = &str> {
        self.rules.iter().map(|rule| rule.tag_id.as_str())
    }

    #[must_use]
    pub const fn version(&self) -> u64 {
        self.version
    }
}

fn evaluate(predicate: &Predicate, ctx: &EntityContext<'_>, store: &TagStore) -> bool {
    match predicate {
        Predicate::SqlContains { needle, .. } => ctx.label.contains(needle),
        Predicate::JsonPathEquals { value, .. } => ctx.label == value,
        Predicate::LabelContains { label: expected } => ctx.label.contains(expected),
        Predicate::FieldEquals { field, value } => {
            field_str(ctx.fields, field).is_some_and(|actual| actual == value)
        }
        Predicate::FieldContains { field, needle } => {
            field_str(ctx.fields, field).is_some_and(|actual| actual.contains(needle.as_str()))
        }
        Predicate::FieldMatchesAny { field, values } => match field_value(ctx.fields, field) {
            Some(Value::String(actual)) => values.iter().any(|value| value == actual),
            Some(Value::Array(items)) => items.iter().any(|item| {
                item.as_str()
                    .is_some_and(|actual| values.iter().any(|value| value == actual))
            }),
            _ => false,
        },
        Predicate::HasTag {
            tag,
            include_descendants,
        } => {
            ctx.active_tags.iter().any(|active| active == tag)
                || (*include_descendants
                    && store
                        .descendants(tag)
                        .iter()
                        .any(|descendant| ctx.active_tags.contains(descendant)))
        }
        Predicate::HasNeighbor {
            edge_type,
            target_tag,
            target_kind,
        } => ctx.neighbors.iter().any(|neighbor| {
            neighbor.edge_type == *edge_type
                && target_tag
                    .as_ref()
                    .is_none_or(|tag| neighbor.tags.contains(tag))
                && target_kind
                    .as_ref()
                    .is_none_or(|kind| neighbor.kind.as_ref() == Some(kind))
        }),
        Predicate::All(predicates) => predicates
            .iter()
            .all(|predicate| evaluate(predicate, ctx, store)),
        Predicate::Any(predicates) => predicates
            .iter()
            .any(|predicate| evaluate(predicate, ctx, store)),
        Predicate::Not(predicate) => !evaluate(predicate, ctx, store),
    }
}

/// Resolve a dotted path (`a.b.c`) into a fields object.
fn field_value<'v>(fields: &'v Value, path: &str) -> Option<&'v Value> {
    let mut current = fields;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

fn field_str<'v>(fields: &'v Value, path: &str) -> Option<&'v str> {
    field_value(fields, path).and_then(Value::as_str)
}

fn validate_rule(rule: &TagRule) -> Result<(), RuleError> {
    if !is_identifier(&rule.rule_id) {
        return Err(RuleError::InvalidRule("rule_id"));
    }
    if !is_tag_id(&rule.tag_id) {
        return Err(RuleError::InvalidRule("tag_id"));
    }
    validate_predicate(&rule.predicate, 1)
}

fn validate_predicate(predicate: &Predicate, depth: usize) -> Result<(), RuleError> {
    if depth > MAX_PREDICATE_DEPTH {
        return Err(RuleError::Recursion);
    }
    match predicate {
        Predicate::SqlContains { sql, needle } => {
            if sql.contains(';') || sql.contains("--") || contains_dangerous_sql_keyword(sql) {
                return Err(RuleError::UnsafeSql);
            }
            if needle.trim().is_empty() {
                return Err(RuleError::InvalidRule("needle"));
            }
        }
        Predicate::JsonPathEquals { path, value } => {
            if !path.starts_with("$.") || path.contains("..") || path.chars().any(char::is_control)
            {
                return Err(RuleError::InvalidRule("json_path"));
            }
            if value.trim().is_empty() {
                return Err(RuleError::InvalidRule("value"));
            }
        }
        Predicate::LabelContains { label } => {
            if label.trim().is_empty() || label.chars().any(char::is_control) {
                return Err(RuleError::InvalidRule("label"));
            }
        }
        Predicate::FieldEquals { field, value } => {
            validate_field_path(field)?;
            if value.trim().is_empty() {
                return Err(RuleError::InvalidRule("value"));
            }
        }
        Predicate::FieldContains { field, needle } => {
            validate_field_path(field)?;
            if needle.trim().is_empty() {
                return Err(RuleError::InvalidRule("needle"));
            }
        }
        Predicate::FieldMatchesAny { field, values } => {
            validate_field_path(field)?;
            if values.is_empty() || values.iter().any(|value| value.trim().is_empty()) {
                return Err(RuleError::InvalidRule("values"));
            }
        }
        Predicate::HasTag { tag, .. } => {
            if !is_tag_id(tag) {
                return Err(RuleError::InvalidRule("tag"));
            }
        }
        Predicate::HasNeighbor {
            edge_type,
            target_tag,
            target_kind,
        } => {
            if !is_graph_id(edge_type) {
                return Err(RuleError::InvalidRule("edge_type"));
            }
            if target_tag.as_deref().is_some_and(|tag| !is_tag_id(tag)) {
                return Err(RuleError::InvalidRule("target_tag"));
            }
            if target_kind
                .as_deref()
                .is_some_and(|kind| !is_identifier(kind))
            {
                return Err(RuleError::InvalidRule("target_kind"));
            }
        }
        Predicate::All(predicates) | Predicate::Any(predicates) => {
            if predicates.is_empty() {
                return Err(RuleError::InvalidRule("empty_combinator"));
            }
            for inner in predicates {
                validate_predicate(inner, depth + 1)?;
            }
        }
        Predicate::Not(inner) => validate_predicate(inner, depth + 1)?,
    }
    Ok(())
}

fn validate_field_path(path: &str) -> Result<(), RuleError> {
    let valid = !path.is_empty()
        && !path.starts_with('.')
        && !path.ends_with('.')
        && !path.contains("..")
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if valid {
        Ok(())
    } else {
        Err(RuleError::InvalidRule("field"))
    }
}

/// Reject a SQL fragment that contains a destructive DML/DDL keyword as a WHOLE
/// word. Tokenizes on non-identifier characters (`[A-Za-z0-9_]` are word chars),
/// so `"drop table x"`, `"(drop"`, and `"x drop"` are all caught while identifiers
/// that merely embed a keyword (`"dropped"`, `"updated_at"`, `"last_update"`) are
/// not. This closes a bypass in the previous space-bounded `" drop "` check, which
/// missed a leading/adjacent keyword such as `"drop table tags"`.
fn contains_dangerous_sql_keyword(sql: &str) -> bool {
    sql.to_ascii_lowercase()
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .any(|token| {
            matches!(
                token,
                "drop"
                    | "delete"
                    | "insert"
                    | "update"
                    | "alter"
                    | "truncate"
                    | "exec"
                    | "merge"
                    | "grant"
                    | "revoke"
            )
        })
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

fn is_graph_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ':' | '.' | '_' | '-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tdw_tags::TagDefinition;

    const NOW: &str = "2026-06-10";

    fn store_with(tags: &[(&str, Option<&str>)]) -> TagStore {
        let mut store = TagStore::default();
        for (tag, parent) in tags {
            store
                .define(TagDefinition {
                    tag_id: (*tag).to_string(),
                    parent: parent.map(ToString::to_string),
                    ttl_days: None,
                })
                .unwrap_or_else(|error| panic!("{tag} should define: {error}"));
        }
        store
    }

    #[test]
    fn apply_at_skips_already_active_tags() {
        let mut tags = store_with(&[("asset:equity", None)]);
        let mut engine = RuleEngine::default();
        engine
            .hot_reload(vec![TagRule {
                rule_id: "equity-symbol".to_string(),
                tag_id: "asset:equity".to_string(),
                predicate: Predicate::LabelContains {
                    label: "AAPL".to_string(),
                },
            }])
            .unwrap_or_else(|error| panic!("rule should reload: {error}"));
        // The entity already actively holds the tag: re-evaluation must not
        // append a duplicate to the append-only store.
        let active = vec!["asset:equity".to_string()];
        let ctx = EntityContext {
            entity_id: "instrument:AAPL",
            label: "AAPL",
            fields: &Value::Null,
            active_tags: &active,
            neighbors: &[],
        };
        let assignments = engine
            .apply_at(&ctx, NOW, &mut tags)
            .unwrap_or_else(|error| panic!("rule should apply: {error}"));
        assert!(assignments.is_empty(), "{assignments:?}");
        assert!(tags.assignments().is_empty(), "no duplicate was appended");
    }

    #[test]
    fn hot_reload_applies_rule_and_rejects_sql_injection() {
        let mut tags = store_with(&[("asset:equity", None)]);
        let mut engine = RuleEngine::default();
        engine
            .hot_reload(vec![TagRule {
                rule_id: "equity-symbol".to_string(),
                tag_id: "asset:equity".to_string(),
                predicate: Predicate::LabelContains {
                    label: "AAPL".to_string(),
                },
            }])
            .unwrap_or_else(|error| panic!("rule should reload: {error}"));
        let assignments = engine
            .apply("instrument:AAPL", "AAPL", NOW, &mut tags)
            .unwrap_or_else(|error| panic!("rule should apply: {error}"));

        assert_eq!(engine.version(), 1);
        // Provenance carries the rule id AND the rule-set version; assigned_at is
        // the injected now, not a hardcoded date.
        assert_eq!(assignments[0].provenance, "rule:equity-symbol@v1");
        assert_eq!(assignments[0].assigned_at, NOW);
        assert_eq!(
            engine.hot_reload(vec![TagRule {
                rule_id: "bad".to_string(),
                tag_id: "asset:equity".to_string(),
                predicate: Predicate::SqlContains {
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
                predicate: Predicate::LabelContains {
                    label: "AAPL".to_string(),
                },
            }]),
            Err(RuleError::InvalidRule("rule_id"))
        );
        engine
            .hot_reload(vec![TagRule {
                rule_id: "missing-tag".to_string(),
                tag_id: "asset:missing".to_string(),
                predicate: Predicate::LabelContains {
                    label: "AAPL".to_string(),
                },
            }])
            .unwrap_or_else(|error| panic!("rule should reload: {error}"));

        let mut tags = TagStore::default();
        assert!(matches!(
            engine.apply("instrument:AAPL", "AAPL", NOW, &mut tags),
            Err(RuleError::Tag(_))
        ));
    }

    #[test]
    fn json_path_equals_predicate_validates_and_matches_on_exact_label() {
        let mut engine = RuleEngine::default();
        assert_eq!(
            engine.hot_reload(vec![TagRule {
                rule_id: "bad-path".to_string(),
                tag_id: "asset:equity".to_string(),
                predicate: Predicate::JsonPathEquals {
                    path: "asset".to_string(),
                    value: "equity".to_string(),
                },
            }]),
            Err(RuleError::InvalidRule("json_path"))
        );
        assert_eq!(
            engine.hot_reload(vec![TagRule {
                rule_id: "empty-value".to_string(),
                tag_id: "asset:equity".to_string(),
                predicate: Predicate::JsonPathEquals {
                    path: "$.asset".to_string(),
                    value: "   ".to_string(),
                },
            }]),
            Err(RuleError::InvalidRule("value"))
        );

        let mut tags = store_with(&[("asset:equity", None)]);
        engine
            .hot_reload(vec![TagRule {
                rule_id: "equity-exact".to_string(),
                tag_id: "asset:equity".to_string(),
                predicate: Predicate::JsonPathEquals {
                    path: "$.asset_class".to_string(),
                    value: "equity".to_string(),
                },
            }])
            .unwrap_or_else(|error| panic!("rule should reload: {error}"));

        let matched = engine
            .apply("instrument:1", "equity", NOW, &mut tags)
            .unwrap_or_else(|error| panic!("apply should succeed: {error}"));
        assert_eq!(matched.len(), 1, "exact label equals value -> assigned");

        let unmatched = engine
            .apply("instrument:2", "equity-ish", NOW, &mut tags)
            .unwrap_or_else(|error| panic!("apply should succeed: {error}"));
        assert!(
            unmatched.is_empty(),
            "superstring must not match (equality, not contains)"
        );
    }

    #[test]
    fn sql_keyword_denylist_catches_leading_keywords_but_not_substrings() {
        let mut engine = RuleEngine::default();
        let rule = |sql: &str| TagRule {
            rule_id: "r".to_string(),
            tag_id: "asset:equity".to_string(),
            predicate: Predicate::SqlContains {
                sql: sql.to_string(),
                needle: "AAPL".to_string(),
            },
        };

        assert_eq!(
            engine.hot_reload(vec![rule("drop table tags")]),
            Err(RuleError::UnsafeSql)
        );
        assert_eq!(
            engine.hot_reload(vec![rule("select * from (delete from t)")]),
            Err(RuleError::UnsafeSql)
        );
        engine
            .hot_reload(vec![rule("select dropped_count, updated_at from t")])
            .unwrap_or_else(|error| panic!("benign keyword-substring sql should pass: {error:?}"));
    }

    #[test]
    fn structured_field_predicates_match_dotted_paths_and_arrays() {
        let mut tags = store_with(&[("asset:equity", None), ("region:us", None)]);
        let mut engine = RuleEngine::default();
        engine
            .hot_reload(vec![
                TagRule {
                    rule_id: "by-class".to_string(),
                    tag_id: "asset:equity".to_string(),
                    predicate: Predicate::FieldEquals {
                        field: "meta.asset_class".to_string(),
                        value: "equity".to_string(),
                    },
                },
                TagRule {
                    rule_id: "by-venue".to_string(),
                    tag_id: "region:us".to_string(),
                    predicate: Predicate::FieldMatchesAny {
                        field: "venues".to_string(),
                        values: vec!["XNAS".to_string(), "XNYS".to_string()],
                    },
                },
            ])
            .unwrap_or_else(|error| panic!("rules should reload: {error}"));

        let fields = json!({
            "meta": { "asset_class": "equity" },
            "venues": ["XETR", "XNAS"],
        });
        let ctx = EntityContext {
            entity_id: "instrument:AAPL",
            label: "Apple",
            fields: &fields,
            active_tags: &[],
            neighbors: &[],
        };
        let assigned = engine
            .apply_at(&ctx, NOW, &mut tags)
            .unwrap_or_else(|error| panic!("apply_at should succeed: {error}"));
        assert_eq!(assigned.len(), 2, "dotted path + array intersection match");

        // A miss on every field: nothing assigned.
        let other = json!({ "meta": { "asset_class": "bond" }, "venues": ["XLON"] });
        let missed = engine
            .apply_at(
                &EntityContext {
                    fields: &other,
                    entity_id: "instrument:GILT",
                    ..ctx
                },
                NOW,
                &mut tags,
            )
            .unwrap_or_else(|error| panic!("apply_at should succeed: {error}"));
        assert!(missed.is_empty());
    }

    #[test]
    fn has_tag_subsumption_and_neighbor_predicates_use_context() {
        let mut tags = store_with(&[
            ("asset:any", None),
            ("asset:equity", Some("asset:any")),
            ("risk:exposed", None),
        ]);
        let mut engine = RuleEngine::default();
        engine
            .hot_reload(vec![TagRule {
                rule_id: "exposure".to_string(),
                tag_id: "risk:exposed".to_string(),
                predicate: Predicate::All(vec![
                    Predicate::HasTag {
                        tag: "asset:any".to_string(),
                        include_descendants: true,
                    },
                    Predicate::HasNeighbor {
                        edge_type: "listed_on".to_string(),
                        target_tag: None,
                        target_kind: Some("venue".to_string()),
                    },
                ]),
            }])
            .unwrap_or_else(|error| panic!("rule should reload: {error}"));

        let active = vec!["asset:equity".to_string()]; // a DESCENDANT of asset:any
        let neighbors = vec![NeighborView {
            edge_type: "listed_on".to_string(),
            entity_id: "venue:XNAS".to_string(),
            kind: Some("venue".to_string()),
            tags: Vec::new(),
        }];
        let ctx = EntityContext {
            entity_id: "instrument:AAPL",
            label: "Apple",
            fields: &Value::Null,
            active_tags: &active,
            neighbors: &neighbors,
        };
        let assigned = engine
            .apply_at(&ctx, NOW, &mut tags)
            .unwrap_or_else(|error| panic!("apply_at should succeed: {error}"));
        assert_eq!(assigned.len(), 1, "subsumed tag + kind-matched neighbor");

        // Without the neighbor the All() fails; Not() inverts.
        let no_neighbors = EntityContext {
            neighbors: &[],
            ..ctx
        };
        assert!(
            engine
                .apply_at(&no_neighbors, NOW, &mut tags)
                .unwrap_or_else(|error| panic!("apply_at should succeed: {error}"))
                .is_empty()
        );
    }

    #[test]
    fn combinator_depth_and_shape_are_validated() {
        let mut engine = RuleEngine::default();
        let leaf = || Predicate::LabelContains {
            label: "x".to_string(),
        };
        // Depth 5 (Not>Not>Not>Not>leaf counts 1+4) exceeds MAX_PREDICATE_DEPTH=4.
        let deep = Predicate::Not(Box::new(Predicate::Not(Box::new(Predicate::Not(
            Box::new(Predicate::Not(Box::new(leaf()))),
        )))));
        assert_eq!(
            engine.hot_reload(vec![TagRule {
                rule_id: "deep".to_string(),
                tag_id: "a:b".to_string(),
                predicate: deep,
            }]),
            Err(RuleError::Recursion)
        );
        assert_eq!(
            engine.hot_reload(vec![TagRule {
                rule_id: "empty".to_string(),
                tag_id: "a:b".to_string(),
                predicate: Predicate::All(Vec::new()),
            }]),
            Err(RuleError::InvalidRule("empty_combinator"))
        );
        assert_eq!(
            engine.hot_reload(vec![TagRule {
                rule_id: "bad-field".to_string(),
                tag_id: "a:b".to_string(),
                predicate: Predicate::FieldEquals {
                    field: "a..b".to_string(),
                    value: "x".to_string(),
                },
            }]),
            Err(RuleError::InvalidRule("field"))
        );
    }

    #[test]
    fn v1_rule_serde_shapes_still_deserialize() {
        // Persisted v1 rules must keep loading: the enum was renamed to
        // Predicate but the variant/field shapes are unchanged.
        let rule: TagRule = serde_json::from_value(json!({
            "rule_id": "legacy",
            "tag_id": "asset:equity",
            "predicate": { "LabelContains": { "label": "AAPL" } },
        }))
        .expect("v1 rule json should deserialize");
        assert_eq!(
            rule.predicate,
            Predicate::LabelContains {
                label: "AAPL".to_string(),
            }
        );
    }
}
