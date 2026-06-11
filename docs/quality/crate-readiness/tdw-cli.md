# tdw-cli Readiness Worksheet

Owner tranche: G007-client-service-mcp-acp-runtime-and-worker-crates - Client, Service, MCP, ACP, Runtime, and Worker Crates.

## Baseline Inventory

- Manifest: crates\tdw-cli\Cargo.toml
- Target kinds: bin
- Local dependencies: tdw-service-api
- External dependencies: none
- Dev dependencies: none
- Reverse local dependencies: none
- Feature flags: none
- Test attributes detected: 0
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 2 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed.
- [x] Dependency direction reviewed.
- [x] Feature flags reviewed or marked not applicable.
- [x] Public API and error contracts reviewed.
- [x] Runtime behavior reviewed.
- [x] Tests and coverage evidence recorded.
- [x] Docs and examples reviewed.
- [x] Surface wiring reviewed where applicable.
- [x] Scaffold, dead-code, and fallback signals classified.
- [x] Security and reliability risks reviewed.

## Findings

- Binary delegates provider fetch and client-event evidence to the daemon; it does not carry independent business logic.
- Error handling exits non-zero on service failure and reports optional client-event sample failures to stderr.
- Scan signals are intentional sample/error-output calls; no stub, copied FinX-XR, OpenBB, or hidden fallback path was found.

## WS4 / G013 update (OpenBB CLI command-tree parity)

The CLI gained a **catalog-derived command tree** (gap-matrix **L5.3**), so its
facts changed materially:

- Dependencies now include `clap` (pure-Rust workspace dep, default features),
  `tdw-endpoint-catalog`, `tdw-protocol`, `tdw-core`, and `tdw-domain` —
  read-only consumption of the catalog/protocol; no `tdw-service-api`,
  `tdw-endpoint-catalog`, or `tdw-protocol` types were modified.
- New modules: `tree` (catalog → clap command tree; schema → args), `params`
  (pure `ArgMatches` → `Op::FetchData` params builder), `render` (aligned table /
  JSON / CSV export + RFC-4180 escaping), `routine` (local-file event-spine
  record/run/list under `.tdw/routines/<name>.jsonl`).
- In-crate unit tests now exist (27 total) covering tree generation, schema→arg
  mapping, table/CSV rendering, and routine round trips; the live daemon
  end-to-end remains covered by the offline `--smoke` path (no daemon in CI).
- Legacy entrypoints (`run-query`, `--smoke`, `kg reindex`, the default shutdown
  probe) are dispatched ahead of clap and are unchanged.
- Quickstart documented at `docs/products/cli.md`.
- Export scope is CSV + JSON only; XLSX/Parquet stay with L5.4.

## Verification

- Focused G007 command passed: `cargo test -p tdw-acp -p tdw-app-client -p tdw-app-server -p tdw-exec -p tdw-runtime -p tdw-service-api -p tdw-tui -p tdw-cli -p tdw-mcp -p tdw-service -p tdw-worker`.

## Verdict

Ready with follow-ups. The CLI is a thin service API entrypoint; richer argument parsing is future product work, not missing bootstrap wiring.

## Smoke Evidence (G009)

Participates in the [end-to-end functional smoke](../end-to-end-smoke.md). The smoke composition is exercised by:

- `tdw-test-utils::smoke::run_end_to_end_smoke` (library entry)
- `crates/tdw-test-utils/tests/end_to_end_smoke.rs` (integration tests)
- `tdw-service` and `tdw-cli` binaries (programmatic harness output)

Verified with `cargo test -p tdw-test-utils --test end_to_end_smoke` — green.
