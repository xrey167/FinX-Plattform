//! Offline `tdw-tag-rules` example: hot-reload a rule, apply it to a tagged
//! value (writing into a `TagStore`), and show an unsafe SQL rule being rejected.
//!
//! Run with:
//!
//! ```text
//! cargo run -p tdw-tag-rules --example tdw-tag-rules-basic
//! ```

use tdw_tag_rules::{RuleEngine, RuleError, RulePredicate, TagRule};
use tdw_tags::{TagDefinition, TagStore};

fn main() {
    // The taxonomy must define the tag a rule wants to assign.
    let mut tags = TagStore::default();
    tags.define(TagDefinition {
        tag_id: "asset:equity".to_string(),
        parent: None,
        ttl_days: None,
    })
    .expect("tag should define");

    // Hot-reload a single label-matching rule.
    let mut engine = RuleEngine::default();
    engine
        .hot_reload(vec![TagRule {
            rule_id: "equity-symbol".to_string(),
            tag_id: "asset:equity".to_string(),
            predicate: RulePredicate::LabelContains {
                label: "AAPL".to_string(),
            },
        }])
        .expect("rule should reload");

    // Meaningful operation: apply the rule to a tagged value.
    let assigned = engine
        .apply("instrument:AAPL", "AAPL", &mut tags)
        .expect("rule should apply");
    println!(
        "engine version {} produced {} assignment(s)",
        engine.version(),
        assigned.len()
    );
    println!("provenance: {}", assigned[0].provenance);

    // Unsafe SQL predicates are rejected at load time.
    let result = engine.hot_reload(vec![TagRule {
        rule_id: "bad".to_string(),
        tag_id: "asset:equity".to_string(),
        predicate: RulePredicate::SqlContains {
            sql: "select * from tags; drop table tags".to_string(),
            needle: "AAPL".to_string(),
        },
    }]);
    println!(
        "unsafe SQL rejected: {}",
        result == Err(RuleError::UnsafeSql)
    );
}
