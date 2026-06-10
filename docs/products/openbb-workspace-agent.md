# OpenBB Workspace Agent (custom copilot)

> **Clean-room note:** The `agents.json` document and the `POST /query` SSE
> copilot protocol implemented here are derived from **public** OpenBB Workspace
> developer documentation and the `openbb-ai` SDK reference only
> (`docs.openbb.co/workspace/copilots`, `docs.openbb.co/workspace/developers`).
> No OpenBB source code was consulted.

This product surface makes the trading-data-warehouse's agents callable **from
OpenBB Workspace as a custom copilot** (`pro.openbb.co`). It serves the two
endpoints Workspace expects from a copilot backend, implementing the published
`openbb-ai` contract as a thin bridge over FinX's existing agent / language-model
machinery.

## What it serves

| Method   | Path           | Returns                                                  |
|----------|----------------|---------------------------------------------------------|
| `GET`    | `/agents.json` | The copilot discovery document (one default copilot)     |
| `POST`   | `/v1/query`    | A `QueryRequest`; responds with a typed SSE event stream |
| `OPTIONS`| either         | CORS preflight                                           |

- **Mapping** lives in `crates/tdw-openbb-agent` (pure, no I/O): the tolerant
  `QueryRequest` / `Widget` / `Message` serde types, the `SseEvent` builder with
  `to_sse_frame()`, the `agents.json` builder (`default_manifest`), prompt
  assembly (`assemble_chat_request`), the two-request decision
  (`needs_widget_data`), and the event sequencer (`answer`).
- **Transport** lives in `crates/tdw-app-server/src/agent_route.rs`
  (`serve_agent_http`), a hand-rolled HTTP/1.1 + SSE surface (no axum/hyper) that
  extends the WSB1 Workspace family — same listener style, same CORS + optional
  `X-TDW-API-KEY` auth posture.
- **Model wiring** lives in `crates/tdw-service-api/src/agent_bridge.rs`
  (`AgentBridgeState`), which implements the transport's `AgentBridgeHandler`
  seam by driving a `StreamingLanguageModel` (the offline `StubLanguageModel` by
  default; inject a live client behind the daemon's credential gates).

## The SSE event vocabulary

Each frame is `event: <type>\ndata: <json>\n\n`. The implemented events match the
documented `openbb-ai` vocabulary:

| `event:` name      | `data` payload                                                         |
|--------------------|------------------------------------------------------------------------|
| `message_chunk`    | `{ "delta": "<text>" }` — one incremental answer fragment               |
| `reasoning_step`   | `{ "event_type": "INFO\|SUCCESS\|WARNING\|ERROR", "message", "details"? }` |
| `table`            | `{ "name", "description"?, "data": [{row}] }`                           |
| `chart`            | `{ "name"?, "type": "line\|bar\|scatter\|pie\|donut", "data", "x_key", "y_keys" }` |
| `citations`        | `{ "citations": [{ "source_widget_id", "input_arguments"? }] }`         |
| `get_widget_data`  | `{ "data_sources": [{ "widget_id", "input_arguments"? }] }`             |

### Documented ambiguities (defensible readings)

The public docs are ambiguous on two field names; the chosen reading is
documented inline in `crates/tdw-openbb-agent/src/event.rs`:

- **`message_chunk` payload key.** The SDK has used both a bare-string `data`
  and a `{ "delta": … }` object across versions. The bridge emits the object
  form (`{ "delta": "<text>" }`) — unambiguous to parse and matching the most
  recent SDK helper.
- **`get_widget_data` container key.** The published examples name the array
  `data_sources`; each entry carries `widget_id` plus `input_arguments` (the
  same names the `citations` event uses, for cross-event consistency).

## The stateless two-request widget-data pattern

A copilot request carries widget **descriptions** (name, description, current
params) but not widget **data**. When the user's question references a primary
widget whose data is not yet folded in, the bridge:

1. **Leg 1.** Emits an opening `reasoning_step` (INFO), then a `get_widget_data`
   event naming the primary widget (with its current params as
   `input_arguments`), and **closes the stream**.
2. The Workspace frontend fetches that widget's data and re-POSTs the *whole*
   conversation with an appended `role: "tool"` message carrying the result.
3. **Leg 2.** The bridge folds the tool result into the model context, streams
   the answer as `message_chunk`s, and closes with a `SUCCESS` `reasoning_step`
   and a `citations` event attributing the answer to the widget.

This is stateless: each leg is an independent `POST`; the conversation itself
carries the state.

## agents.json content

```json
{
  "finx-copilot": {
    "name": "FinX Copilot",
    "description": "Answers questions grounded in your FinX dashboard widgets, fetching widget data on demand and citing its sources.",
    "endpoints": { "query": "http://127.0.0.1:6900/v1/query" },
    "features": {
      "streaming": true,
      "widget-dashboard-select": true,
      "widget-dashboard-search": false
    }
  }
}
```

The agent id matches the documented `^[a-z0-9-]+$` pattern. v1 exposes exactly
one default copilot; projecting the full `tdw-agent` registry onto `agents.json`
(one entry per registry agent) is a documented follow-up. The `endpoints.query`
URL is composed from the bind address so it points back at the same listener.

## Setup

The agent listener is **off by default** and env-gated. Enable it on the
`tdw-backend` daemon by compiling with the `agent-route` feature and binding an
address:

```sh
cargo run -p tdw-backend --features agent-route --target-dir target
# with, in the environment:
#   TDW_WORKSPACE_BIND=127.0.0.1:6900     # the agent + workspace family bind
#   TDW_WORKSPACE_API_KEY=<optional shared key>   # fail-closed when set
#   TDW_WORKSPACE_CORS_ORIGINS=https://pro.openbb.co  # optional override
```

CORS + auth are shared with the WSB1 workspace family: the default origins are
`https://pro.openbb.co` plus the documented local dev origins; setting
`TDW_WORKSPACE_API_KEY` requires every request to present a matching
`X-TDW-API-KEY` header (constant-time compared, fail-closed). Bind a loopback
address unless you front the daemon with a token / mTLS / reverse-proxy layer.

The offline default uses the deterministic `StubLanguageModel`, so the surface
runs end-to-end with no network or credentials (useful for the manual checklist
below and for CI). Inject a live streaming model behind the existing credential
gates to answer against a real LLM.

## Manual Workspace interop checklist

1. **Start the daemon** with `--features agent-route` and `TDW_WORKSPACE_BIND`
   set (see Setup). Confirm `GET http://127.0.0.1:6900/agents.json` returns the
   one-copilot document above.
2. **Register the copilot in Workspace.** In `pro.openbb.co`, add a custom
   copilot backend pointing at `http://127.0.0.1:6900` (the base URL serving
   `/agents.json`). Confirm "FinX Copilot" appears in the copilot picker.
3. **Ask a no-widget question** (e.g. "What is a P/E ratio?"). Confirm the
   answer streams in token-by-token (a `reasoning_step` followed by
   `message_chunk`s) and no widget fetch is requested.
4. **Attach a primary widget** (e.g. the `equity/price/historical` chart for
   AAPL) and ask about it ("How did AAPL do?"). Confirm the copilot shows a
   "Fetching data for widget …" step, the frontend fetches the widget data, and
   a second request streams the grounded answer ending with a **citation** to
   that widget.
5. **Auth (optional).** With `TDW_WORKSPACE_API_KEY` set, confirm a request
   without the `X-TDW-API-KEY` header is rejected `401`, and the configured key
   is accepted.

## Tests

- Unit (`crates/tdw-openbb-agent`): serde round-trips from doc-fixture JSON,
  prompt assembly, the two-request folding decision, golden SSE frames, citation
  building, and the end-to-end event sequencer against a local echo stub.
- E2E (`crates/tdw-service-api/tests/agent_route_e2e.rs`): the real
  `serve_agent_http` listener + the offline `StubLanguageModel` —
  `agents.json` parses; `POST /query` streams `reasoning_step` + `message_chunk`;
  the two-request widget-data round trip closes with `citations`; CORS preflight
  and the `X-TDW-API-KEY` 401 are exercised at the transport.
