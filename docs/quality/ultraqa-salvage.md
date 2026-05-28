# UltraQA Salvage

This note records the useful payload recovered from the local
`salvage/ultraqa-recover` branch after the cleanup pass.

The original UltraQA report and CSV were generated from a stale static-analysis
snapshot and were not landed. The salvaged value is the characterization test
set, rebased onto current `main` and revalidated locally.

## Imported Test Coverage

- `tdw-actor/tests/actor_context.rs`
- `tdw-bus/tests/bus_capacity.rs`
- `tdw-cdc/tests/cdc_offsets.rs`
- `tdw-config/tests/layers.rs`
- `tdw-core/tests/trait_contracts.rs`
- `tdw-domain/tests/validate_per_bom.rs`
- `tdw-event/tests/envelope_chain.rs`
- `tdw-graph/tests/graph_traversal.rs`
- `tdw-hooks/tests/registry_contracts.rs`
- `tdw-mask/tests/mask_modes.rs`
- `tdw-outbox/tests/outbox_lifecycle.rs`
- `tdw-protocol/tests/contract.rs`
- `tdw-runtime/tests/runner_errors.rs`
- `tdw-session/tests/durability.rs`
- `tdw-snapshot/tests/snapshot_versions.rs`
- `tdw-spatial/tests/spatial_bounds.rs`

## Rebase Fixes

- Updated `tdw-core` and `tdw-runtime` tests for the current
  `Credentials::anthropic_api_key` field.
- Replaced a `Result::expect_err` assertion on `run_streaming` with an explicit
  match because the success stream type does not implement `Debug`.
- Preserved current `tdw-snapshot` Postgres feature wiring while adding the
  `serde_json` dev dependency needed by the imported integration test.

## Validation

Run from `work/cleanup-ultraqa-salvage`:

```powershell
$env:CARGO_TARGET_DIR = Join-Path $env:TEMP 'finx-ultraqa-target'
cargo +stable test -p tdw-actor -p tdw-bus -p tdw-cdc -p tdw-config -p tdw-core -p tdw-domain -p tdw-event -p tdw-graph -p tdw-hooks -p tdw-mask -p tdw-outbox -p tdw-protocol -p tdw-runtime -p tdw-session -p tdw-snapshot -p tdw-spatial
```

Result: PASS.
