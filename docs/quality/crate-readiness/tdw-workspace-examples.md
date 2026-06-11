# tdw-workspace-examples Readiness Worksheet

Generated during the G017 "WSB4 Examples Suite" landing, which introduced the
progressive, offline-runnable OpenBB-Workspace example suite under
`examples/workspace/`.

## Evidence Snapshot

- Manifest: `examples/workspace/Cargo.toml`.
- Targets: lib (the per-example library functions), seven example `[[bin]]`s
  (`ws-10-backend-minimal` … `ws-70-agent-charts-tables`, each a thin `main()`
  in its own `NN-name/` directory), and one integration test
  (`tests/examples_e2e.rs`).
- Local deps: `tdw-service-api` (feature `agent-route`), `tdw-app-server`
  (feature `agent-route`), `tdw-widgets`, `tdw-openbb-agent`, `tdw-eval-runner`,
  `tdw-charting`, `tdw-domain` (plus `serde_json`, `tokio`).
- Reverse deps: none — nothing in the workspace depends on the example suite.
- Features: none.
- Tests: 10 integration tests in `tests/examples_e2e.rs`, one or more per
  example, that boot each example's server path (`AppState::in_memory_for_tests`
  behind the REST / workspace / agent transports) or drive its pure builder and
  assert golden outputs: the hand-written widgets.json/data + 404; the derived
  widgets.json widget count (60+) and offline fileset widget-data rows; the
  curated apps.json default app + the custom two-tab app; the copilot
  agents.json + the no-widget reasoning/message_chunk stream; the grounded
  reasoning + citations transcript; the two-request get_widget_data round trip;
  and the table + chart SSE artifacts plus the reused Plotly candlestick spec.
- Docs/examples: this worksheet, a top-level `examples/workspace/README.md`, a
  per-example `README.md` + `Dockerfile`, module-level docs citing the public
  OpenBB Workspace doc URLs (transitively via the bridge crates), and the
  product-doc links from `docs/products/openbb-parity.md`.

## Release Assessment

- The crate is **not** a runtime component: it is a teaching/verification
  artifact. It performs no I/O beyond loopback HTTP to its own in-process
  servers and reaches no network — every example runs offline (the data
  examples resolve the always-registered `fileset` fixture; the copilot examples
  drive the offline `StubLanguageModel`).
- **Build-time isolation.** The crate is a workspace *member* (so
  `cargo metadata --no-deps` covers it and its CI tests run) but is excluded
  from `default-members`, so a bare `cargo build` / `cargo test --workspace`
  default scope is unchanged and the suite never inflates the warm build the
  rest of the workspace pays. CI builds/tests it explicitly with
  `-p tdw-workspace-examples`.
- Clean-room: every contract the examples author (widgets.json / apps.json /
  the agent SSE vocabulary) is consumed from the existing clean-room bridge
  crates, which project **public** OpenBB Workspace documentation only. The
  example crate adds no new contract types. Because it is excluded from
  `default-members` it is outside the default pedantic/nursery ratchet scope,
  but it carries zero new warnings under `clippy -D warnings` /
  `-W clippy::pedantic -W clippy::nursery` on its own `-p` invocation, and it
  forbids `unsafe`/`unwrap` like the rest of the workspace.
- Any code-level follow-up remains non-blocking unless `fmt`,
  `clippy -D warnings`, the example tests, the clean-room audit, or
  `crate-readiness-check` fails.

## Verdict

Ready with follow-ups. The suite covers the P1 data backend (raw + derived +
apps) and the P2 copilot (echo, reasoning/citations, the two-request widget-data
round trip, and table/chart artifacts). The Python SDK example (`80-python-sdk`)
ships as a standalone script with its own README/Dockerfile and is CI-checked via
`py_compile` in the existing pysdk gate path; running it end to end against a
booted daemon is documented as a manual step. Later stories can broaden the
suite (e.g. a live-model copilot variant, more curated apps) without changing
this membership/build-time posture.
