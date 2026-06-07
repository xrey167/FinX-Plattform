//! Offline `tdw-agent-store` example: an in-memory `AgentStore` round-trip
//! (agent + workflow + eval run) plus a checked-path rejection. No filesystem,
//! no scheduler, no network.
//!
//! ```text
//! cargo run --example tdw_agent_store_basic -p tdw-agent-store
//! ```

use tdw_agent::{
    Adaptivity, EntityMeta, EvalCase, EvalRunRequest, Origin, Source, Tier, WorkflowDefinition,
    WorkflowEdge, WorkflowNode, sample_agent_card,
};
use tdw_agent_store::{AgentStore, StoreError, StoredEvalRun};

fn meta(name: &str) -> EntityMeta {
    EntityMeta::new(
        name,
        name,
        "0.1.0",
        Origin {
            tier: Tier::Domain,
            source: Source::Internal,
        },
        Adaptivity::None,
        false,
    )
}

fn main() {
    let mut store = AgentStore::new();

    // Agent round-trip.
    let card = sample_agent_card();
    store.upsert_agent(card.clone());
    assert_eq!(store.agent("market-researcher"), Some(&card));
    println!("stored agent: {}", card.meta.id);

    // Workflow round-trip (checked path).
    let workflow = WorkflowDefinition {
        meta: meta("research-flow"),
        nodes: vec![
            WorkflowNode {
                node_id: "retrieve".to_string(),
                task: "retrieve".to_string(),
                skill_id: None,
            },
            WorkflowNode {
                node_id: "draft".to_string(),
                task: "draft".to_string(),
                skill_id: None,
            },
        ],
        edges: vec![WorkflowEdge {
            from: "retrieve".to_string(),
            to: "draft".to_string(),
        }],
    };
    store
        .try_upsert_workflow(workflow.clone())
        .expect("valid workflow should store");
    assert_eq!(store.workflow("research-flow"), Some(&workflow));
    println!("stored workflow: research-flow");

    // Eval-run round-trip.
    let run = StoredEvalRun {
        request: EvalRunRequest {
            run_id: "eval-1".to_string(),
            agent_id: "market-researcher".to_string(),
            dataset_id: "golden-market-notes".to_string(),
            cases: vec![EvalCase {
                case_id: "case-1".to_string(),
                prompt: "Summarize AAPL".to_string(),
                expected_refs: Vec::new(),
            }],
        },
        metrics: Vec::new(),
        status: "success".to_string(),
        updated_skills: Vec::new(),
    };
    store.record_eval_run(run.clone());
    assert_eq!(store.eval_run("eval-1"), Some(&run));
    println!("recorded eval run: eval-1 ({})", run.status);

    // Checked path rejects a malformed agent name.
    let mut bad = sample_agent_card();
    bad.meta.base.name = "../agent".to_string();
    assert_eq!(store.try_upsert_agent(bad), Err(StoreError::InvalidAgent));
    println!("malformed agent name rejected, as expected");
}
