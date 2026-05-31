//! Env-gated real-path integration test for [`EvalRunner`].
//!
//! Mirrors the `tdw-llm-anthropic` live-test gating: it runs only when
//! `TDW_LLM_LIVE=1` AND `TDW_ANTHROPIC_API_KEY` are set, and early-returns
//! (cleanly, exit 0) otherwise — so the default offline workspace test set never
//! touches the network. When enabled it drives a real [`AnthropicMessagesModel`]
//! (a real `LanguageModel` impl) through the runner end-to-end and asserts the run
//! persisted with the expected per-case metric shape.
//!
//! The default offline path is covered by the unit tests via `StubLanguageModel`.

use std::sync::Arc;

use tdw_agent::{EvalCase, EvalRunRequest, sample_agent_card};
use tdw_agent_store::AgentStore;
use tdw_eval_runner::EvalRunner;
use tdw_llm_anthropic::AnthropicMessagesModel;

fn live_enabled() -> bool {
    std::env::var("TDW_LLM_LIVE").ok().as_deref() == Some("1")
}

fn api_key() -> Option<String> {
    std::env::var("TDW_ANTHROPIC_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())
}

#[test]
fn live_runner_executes_case_through_real_model_when_env_var_set() {
    if !live_enabled() {
        eprintln!("TDW_LLM_LIVE != 1; skipping live eval-runner integration test");
        return;
    }
    if api_key().is_none() {
        eprintln!("TDW_ANTHROPIC_API_KEY not set; skipping live eval-runner integration test");
        return;
    }

    let model = AnthropicMessagesModel::new("claude-haiku-4-5-20251001")
        .unwrap_or_else(|error| panic!("model id should be valid: {error}"));
    let runner = EvalRunner::new(Arc::new(model));

    let mut store = AgentStore::new();
    let card = sample_agent_card();
    let agent_id = card.meta.id.clone();
    let expected_refs = card.content_refs.clone();
    store.upsert_agent(card);

    let outcome = runner.run(
        EvalRunRequest {
            run_id: "live-eval-1".to_string(),
            agent_id,
            dataset_id: "golden-market-notes".to_string(),
            cases: vec![EvalCase {
                case_id: "case-1".to_string(),
                prompt: "Summarize AAPL".to_string(),
                expected_refs,
            }],
        },
        &mut store,
    );

    assert_eq!(outcome.run_id, "live-eval-1");
    assert!(
        outcome
            .metrics
            .iter()
            .any(|metric| metric.metric_name == "pass_rate"),
        "live run must report a pass_rate metric: {outcome:?}"
    );
    assert!(
        store.eval_run("live-eval-1").is_some(),
        "live run must persist to the store"
    );
}
