# OpenBB Workspace Examples

A progressive, **offline-runnable** example suite that teaches the FinX <->
OpenBB Workspace surface, from the raw data-backend contract up through a
streaming copilot. Every example runs network-free and is verified in CI against
the in-memory daemon.

## How it is structured

All examples live in **one** workspace member crate (`tdw-workspace-examples`)
so the suite costs the readiness machinery exactly one crate and never inflates
the default build:

- Each example is a thin `main()` in its own `NN-name/` directory (alongside that
  example's `README.md` and `Dockerfile`), over a small library function in
  `src/`.
- The CI integration test (`tests/examples_e2e.rs`) calls those same library
  functions directly and boots each Rust example's server path against
  `AppState::in_memory_for_tests()`, asserting golden outputs — the
  repo-idiomatic shape from `tdw-service-api/tests/*_e2e.rs`.
- The crate is a workspace *member* (so `cargo metadata` covers it and its tests
  run in CI) but is excluded from `default-members`, so a bare `cargo build` /
  `cargo test --workspace` default scope — and warm build time — is unchanged.
  Build and test it explicitly:

  ```sh
  cargo build -p tdw-workspace-examples --target-dir target
  cargo test  -p tdw-workspace-examples --target-dir target
  ```

## The examples

| #  | Example | Teaches |
| -- | ------- | ------- |
| 10 | [`10-backend-minimal`](10-backend-minimal/) | The raw `widgets.json` + data-endpoint contract, hand-rolled over a tiny HTTP server (no daemon). |
| 20 | [`20-backend-derived`](20-backend-derived/) | The real backend: boot the in-memory daemon and serve the catalog-derived `widgets.json` (60+ widgets) + offline `widget-data`. |
| 30 | [`30-backend-app`](30-backend-app/) | `apps.json`: the curated FinX Market Overview app + a custom two-tab app. |
| 40 | [`40-agent-echo`](40-agent-echo/) | The minimal copilot: `agents.json` + a streamed answer from the offline stub model. |
| 50 | [`50-agent-reasoning-citations`](50-agent-reasoning-citations/) | `reasoning_step` events + a `citations` event attributing the answer to a widget. |
| 60 | [`60-agent-widget-data`](60-agent-widget-data/) | The stateless two-request `get_widget_data` round trip, scripted end to end. |
| 70 | [`70-agent-charts-tables`](70-agent-charts-tables/) | `table` + `chart` SSE artifacts from widget data (reusing the shared charting builders). |
| 80 | [`80-python-sdk`](80-python-sdk/) | Driving the REST `/api/v1` surface via the checked-in `finx_platform` Python SDK. |

Run any Rust example with its bin name, e.g.:

```sh
cargo run -p tdw-workspace-examples --bin ws-20-backend-derived --target-dir target
```

## Offline by construction

- The data examples (10/20/30) resolve the always-registered `fileset` fixture,
  so `equity/price/historical` returns rows with no provider key.
- The copilot examples (40–70) drive the deterministic, offline
  `StubLanguageModel` — no network, no credentials.
- Example 80 is the only one that needs a separately-started daemon for an
  end-to-end run; CI verifies it compiles (`py_compile`) and the run-it-yourself
  steps are in its README.
