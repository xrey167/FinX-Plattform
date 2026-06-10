# tdw-openbb-agent Readiness Worksheet

Generated during the G008 "WSB2 Agent Protocol Bridge" landing, which introduced
the OpenBB Workspace **agent/copilot** bridge crate (the request/response mapping
layer that makes registry agents callable *from* Workspace as custom copilots).

## Evidence Snapshot

- Manifest: `crates/tdw-openbb-agent/Cargo.toml`.
- Targets: lib (unit tests inline).
- Local deps: `tdw-llm` (for `ChatRequest` / `StreamingLanguageModel`), plus
  `serde`, `serde_json`.
- Reverse deps: `tdw-app-server` (optional, via its `agent-route` feature — the
  transport parses requests into these types and writes the SSE frames they
  render) and `tdw-service-api` (optional, via its `agent-route` feature — the
  `AgentBridgeState` drives the pure `answer` sequencer).
- Features: none.
- Tests: 31 unit tests:
  - `request` — `QueryRequest` / `Widget` / `Message` serde round-trips from
    doc-fixture JSON, tolerant unknown-field/unknown-role handling, `tool`
    structured-content rendering, id preference, message-text extraction.
  - `event` — golden SSE frames for `message_chunk` / `reasoning_step` /
    `get_widget_data` / `citations` / `table` / `chart`, the `citations`
    omitted-argument shape, and the always-blank-line framing invariant.
  - `prompt` — system-message + widget-description assembly, `tool`-result
    folding with the `Widget data:` prefix, role mapping, empty-turn dropping,
    token-budget threading.
  - `decision` — `needs_widget_data` first-leg vs. second-leg (tool-result)
    behaviour, no-widget and empty-param cases, id-less widget skipping.
  - `manifest` — `agents.json` document shape, the `^[a-z0-9-]+$` agent-id, and
    the hyphenated capability keys.
  - `drive` — the end-to-end two-request flow against a local echo stub (first
    leg emits `get_widget_data` + closes; second leg streams chunks + emits
    `citations`; no-widget answers directly; chunks reconstruct the answer).
- Docs/examples: this worksheet, module-level docs citing the public OpenBB
  Workspace copilot doc URLs, and the product doc
  `docs/products/openbb-workspace-agent.md`.

## Release Assessment

- The crate is a pure, offline, deterministic **mapping**: it performs no I/O
  and enforces no policy. It serializes the OpenBB Workspace copilot contract
  (`agents.json` + the `POST /query` SSE event vocabulary) and turns a parsed
  request plus a `StreamingLanguageModel` into the ordered events the transport
  writes. The transport (`tdw-app-server/agent-route`) owns the listener, CORS,
  auth, and the socket writes.
- Clean-room: every contract type is a projection of **public** OpenBB Workspace
  copilot documentation and the `openbb-ai` SDK reference only — no OpenBB source
  code was consulted. Doc URLs are cited in the module docs. Where the public
  docs are ambiguous (the `message_chunk` payload key, the `get_widget_data`
  container key), the chosen reading is documented inline in `event.rs`. The
  crate compiles by default, so it is in the pedantic/nursery ratchet scope; it
  carries zero new warnings.
- SSE frames are pinned by golden unit tests, so any drift in the event shapes
  surfaces as a reviewable diff.
- Any code-level follow-up remains non-blocking unless `fmt`,
  `clippy -D warnings`, tests, the clean-room audit, `catalog-check`, or
  `crate-readiness-check` fails.

## Verdict

Ready with follow-ups. v1 exposes exactly **one** default copilot (the
registry-to-`agents.json` projection — one entry per registry agent — is a
documented follow-up). The agent loop is a single-pass answer: it streams the
model's text, and on a primary widget with absent data it runs the stateless
two-request `get_widget_data` pattern and answers with citations on the
follow-up. Richer tool-calling, multi-widget fan-out, and table/chart artifact
emission are additive follow-ups built on the same `SseEvent` vocabulary.
