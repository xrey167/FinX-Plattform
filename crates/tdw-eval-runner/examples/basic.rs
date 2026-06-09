//! Offline `tdw-eval-runner` example: run an eval through the deterministic
//! `StubLanguageModel` and read the persisted metrics. No network.
//!
//! ```text
//! cargo run --example tdw_eval_runner_basic -p tdw-eval-runner
//! ```

use std::sync::Arc;

use tdw_agent::{ContentKind, ContentRef, EvalCase, EvalRunRequest, sample_agent_card};
use tdw_agent_store::AgentStore;
use tdw_eval_runner::{EvalRunner, StubLanguageModel};

fn main() {
    let mut store = AgentStore::new();
    store.upsert_agent(sample_agent_card());

    let runner = EvalRunner::new(Arc::new(StubLanguageModel));

    // Case 1 passes: its expected ref is the agent's own grounding ref, which the
    // echo stub surfaces. Case 2 fails: its expected ref is never present.
    let outcome = runner.run(
        EvalRunRequest {
            run_id: "eval-1".to_string(),
            agent_id: "market-researcher".to_string(),
            dataset_id: "golden-market-notes".to_string(),
            cases: vec![
                EvalCase {
                    case_id: "case-pass".to_string(),
                    prompt: "Summarize AAPL".to_string(),
                    expected_refs: vec![ContentRef {
                        uri: "tdw://docs/research-template".to_string(),
                        kind: ContentKind::Prompt,
                        checksum: None,
                        tags: Vec::new(),
                    }],
                },
                EvalCase {
                    case_id: "case-fail".to_string(),
                    prompt: "Summarize MSFT".to_string(),
                    expected_refs: vec![ContentRef {
                        uri: "tdw://docs/never-mentioned".to_string(),
                        kind: ContentKind::Prompt,
                        checksum: None,
                        tags: Vec::new(),
                    }],
                },
            ],
        },
        &mut store,
    );

    assert_eq!(outcome.status, "success");
    for metric in &outcome.metrics {
        println!("{} = {}", metric.metric_name, metric.metric_value);
    }

    // The run was persisted with its three metrics.
    assert_eq!(
        store.eval_run("eval-1").map(|run| run.metrics.len()),
        Some(3)
    );
    println!("persisted run eval-1 with status {}", outcome.status);
}
