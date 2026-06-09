//! Offline `tdw-agent` example: validate the sample agent card, validate a
//! workflow DAG (topological order), and emit the self-describing resource
//! registry. Fully in-memory — no network, no filesystem.
//!
//! ```text
//! cargo run --example tdw_agent_basic -p tdw-agent
//! ```

use tdw_agent::{
    Adaptivity, EntityMeta, Origin, Source, Tier, WorkflowDefinition, WorkflowEdge, WorkflowNode,
    resource_definitions, sample_agent_card, validate_agent_card_contract,
    validate_workflow_contract,
};

fn main() {
    // 1) The sample agent card validates against the contract.
    let card = sample_agent_card();
    validate_agent_card_contract(&card).expect("sample card should validate");
    assert_eq!(card.meta.id, "market-researcher");
    println!("agent: {} ({} skill(s))", card.meta.id, card.skills.len());

    // 2) A small workflow validates and yields a topological order.
    let meta = EntityMeta::new(
        "research-flow",
        "research-flow",
        "0.1.0",
        Origin {
            tier: Tier::Domain,
            source: Source::Internal,
        },
        Adaptivity::None,
        false,
    );
    let workflow = WorkflowDefinition {
        meta,
        nodes: vec![
            WorkflowNode {
                node_id: "retrieve".to_string(),
                task: "retrieve".to_string(),
                skill_id: None,
            },
            WorkflowNode {
                node_id: "draft".to_string(),
                task: "draft".to_string(),
                skill_id: Some("research.note".to_string()),
            },
        ],
        edges: vec![WorkflowEdge {
            from: "retrieve".to_string(),
            to: "draft".to_string(),
        }],
    };
    let order = validate_workflow_contract(&workflow).expect("workflow should validate");
    assert_eq!(order, vec!["retrieve".to_string(), "draft".to_string()]);
    println!("workflow topological order: {order:?}");

    // 3) The self-describing registry lists one definition per classified kind.
    let definitions = resource_definitions();
    assert!(!definitions.is_empty());
    let with_schema = definitions
        .iter()
        .filter(|definition| definition.spec_schema.is_some())
        .count();
    println!(
        "registry: {} kinds, {with_schema} with a concrete JSON Schema",
        definitions.len()
    );
}
