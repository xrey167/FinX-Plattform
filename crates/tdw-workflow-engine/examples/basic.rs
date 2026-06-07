//! Offline `tdw-workflow-engine` example: build a two-node workflow DAG and
//! compile it into an ordered execution plan.
//!
//! Run with:
//!
//! ```text
//! cargo run -p tdw-workflow-engine --example tdw-workflow-engine-basic
//! ```

use tdw_agent::{
    Adaptivity, EntityMeta, Origin, Source, Tier, WorkflowDefinition, WorkflowEdge, WorkflowNode,
};
use tdw_workflow_engine::WorkflowEngine;

fn main() {
    // Declare a workflow: retrieve -> draft.
    let workflow = WorkflowDefinition {
        meta: EntityMeta::new(
            "research-flow",
            "research-flow",
            "0.1.0",
            Origin {
                tier: Tier::Domain,
                source: Source::Internal,
            },
            Adaptivity::Configured,
            false,
        ),
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

    // Meaningful operation: compile the DAG into a topological execution plan.
    let plan = WorkflowEngine::compile(&workflow).expect("workflow should compile");
    println!("workflow {} compiled", plan.workflow_id);
    println!("execution order: {:?}", plan.ordered_node_ids);
}
