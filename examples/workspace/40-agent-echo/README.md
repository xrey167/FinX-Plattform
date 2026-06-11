# 40 — Minimal copilot (echo)

The smallest OpenBB Workspace copilot: a backend that publishes `agents.json`
(who the copilot is, where to query it, what it supports) and answers
`POST /v1/query` with a stream of Server-Sent Events. This example is backed by
the deterministic, offline `StubLanguageModel`, so the whole turn runs with no
network and no credentials.

## What it teaches

- `agents.json`: a one-entry document advertising the copilot, its `streaming`
  feature, and its `query` endpoint.
- The `POST /v1/query` SSE response: a no-widget question streams an opening
  `reasoning_step`, then the answer as a series of `message_chunk` events.

## Run

```sh
cargo run -p tdw-workspace-examples --bin ws-40-agent-echo --target-dir target
```

It prints `agents.json`, asks a no-widget question, and reports the SSE event
order.

## Next

Example 50 adds reasoning narration and citations.
