# tdw-eval-runner — Architecture

## Module map

Single-file crate (`src/lib.rs`), `#![forbid(unsafe_code)]`:

| Item | Role |
| --- | --- |
| `EvalRunner` | Holds `Arc<dyn LanguageModel>`; `run` / `try_run`. |
| `EvalRunOutcome` | `{ run_id, status, metrics }`. |
| `EvalRunError` | `InvalidRequest` / `StoreRejected`. |
| `score_case` | The deterministic containment scorer. |
| `validate_request` | Request hygiene. |
| `StubLanguageModel` | The offline default `LanguageModel` (echo). |

## How the model contract is used

`EvalRunner` consumes the `tdw_llm::LanguageModel` trait — it does not implement
it. It calls `model.complete(ChatRequest)` per case. Because the trait is
`Send + Sync`, the model lives behind `Arc<dyn LanguageModel>` and the runner can
be cloned/shared. The default injected model is `StubLanguageModel`; a live run
swaps in `AnthropicMessagesModel` (or any other impl).

## Per-case request construction

`agent_system_prompt(card)` builds the grounding context: the agent's
title/description, each skill's title/description + input/output JSON Schemas,
and — crucially — the agent's own `content_refs` URIs. `build_chat_request` puts
that as a `System` message followed by the case prompt as a `User` message. The
**expected** refs are *not* injected: the scorer checks whether the model's answer
surfaces them from the grounding context.

## Scoring + aggregation

```
for each case:
   build_chat_request → model.complete
      Ok(resp)  → score_case(case, resp.content)   (containment)
      Err(_)    → false  (a model error is a failed case, not an aborted run)
metrics = [case_count, passed, pass_rate]
status  = pass_rate >= 0.5 ? "success" : "failed"
store.record_eval_run(StoredEvalRun { request, metrics, status, updated_skills: [] })
```

`run` is infallible by design (partial results are still scored + stored).
`try_run` adds `validate_request` up front and confirms persistence
(`StoreRejected` otherwise). Skill-quality feedback is a later, gated backend
pass, so `updated_skills` is left empty here.

## The offline stub

`StubLanguageModel` models a faithful, fully-grounded agent: it echoes back every
message's content (grounding context + prompt) as the assistant reply, with no
network. Because the grounding context contains the agent's reference URIs, a case
whose `expected_refs` are present passes the containment check and one whose ref
is absent fails — both deterministically. The stub still validates the request, so
an empty message surfaces an error (recorded as a failed case).

## Offline / live-test design

- **Offline** — unit tests drive `StubLanguageModel` end to end: a pass+fail
  mixed run (`pass_rate = 0.5`, `success`), an all-fail run (`failed`), the
  no-expected-refs case, and the empty-cases rejection.
- **Live** — `tests/live_real_model.rs` drives a real `AnthropicMessagesModel`
  through the runner, gated on `TDW_LLM_LIVE=1` + `TDW_ANTHROPIC_API_KEY`; absent
  those it returns early, so `cargo test --workspace` stays offline.
