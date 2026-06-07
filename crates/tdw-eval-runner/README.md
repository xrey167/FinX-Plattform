# tdw-eval-runner

Executes agent evaluation cases through a `LanguageModel` and scores them
deterministically.

## Purpose

[`EvalRunner`] owns an `Arc<dyn tdw_llm::LanguageModel>` and, for every
[`tdw_agent::EvalCase`], builds a [`tdw_llm::ChatRequest`] from the agent/skill
grounding context plus the case prompt, calls the model, applies one honest
scoring rule ([`score_case`]), aggregates `case_count`/`passed`/`pass_rate`
metrics, persists the run into a `tdw_agent_store::AgentStore`, and returns the
[`EvalRunOutcome`].

The default model is [`StubLanguageModel`], a deterministic offline echo model —
so eval runs are reproducible in CI with no network. Callers swap in a real
client (Anthropic / OpenAI-compatible) when a live model is configured.

## Feature flags

None. Dependencies: `serde`, `serde_json`, `tdw-agent`, `tdw-agent-store`,
`tdw-llm` (+ `tdw-llm-anthropic` as a dev-dependency for the live test).

## Environment variables

The crate reads no env vars directly. The env-gated **live** integration test
(`tests/live_real_model.rs`) drives a real `AnthropicMessagesModel` and runs only
when both are set:

| Variable | Meaning |
| --- | --- |
| `TDW_LLM_LIVE=1` | Opt in to the live model test. |
| `TDW_ANTHROPIC_API_KEY` | The API key for the real model. |

With these unset the live test early-returns cleanly (no network). The default
offline path is covered by unit tests via `StubLanguageModel`.

## Scoring

`score_case` is a containment check, **not** an LLM judge:

1. The response must be non-empty (after trimming).
2. Every `expected_refs` URI must appear verbatim in the response.

A run is `success` when `pass_rate >= 0.5`, else `failed`.

## Quickstart

```rust
use std::sync::Arc;
use tdw_agent::{EvalCase, EvalRunRequest, sample_agent_card};
use tdw_agent_store::AgentStore;
use tdw_eval_runner::{EvalRunner, StubLanguageModel};

let mut store = AgentStore::new();
store.upsert_agent(sample_agent_card());

let runner = EvalRunner::new(Arc::new(StubLanguageModel));
let outcome = runner.run(
    EvalRunRequest {
        run_id: "eval-1".to_string(),
        agent_id: "market-researcher".to_string(),
        dataset_id: "golden-market-notes".to_string(),
        cases: vec![EvalCase {
            case_id: "case-1".to_string(),
            prompt: "Summarize AAPL".to_string(),
            expected_refs: Vec::new(),
        }],
    },
    &mut store,
);
assert_eq!(outcome.status, "success");
```

## Example

```text
cargo run --example tdw_eval_runner_basic -p tdw-eval-runner
```

`examples/basic.rs` runs an eval through `StubLanguageModel` and reads the
persisted metrics — offline, deterministic.
