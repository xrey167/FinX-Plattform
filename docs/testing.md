# Testing

## Phase-Exit Gates

```powershell
just fmt-check
just lint
just test-unit
just test-integration
just test-property
just test-e2e
just test-adversarial
just schema-sync
just event-schema-check
just bench
just quality-gate-check
just deny
just audit
```

`just coverage` writes `lcov.info`. `just windows-release` validates the
MSVC release profile and is required before a release checkpoint. The generated
quality-gate contract lives at `docs/quality/phase-exit-gates.json`; refresh it
with `just quality-gate` after changing the gate list.

## Test Tiers

- Unit tests live beside implementation code in `mod tests`.
- Integration tests belong under `tests/integration/`.
- Property tests belong under `tests/property/`.
- End-to-end tests belong under `tests/e2e/` and should avoid billed APIs by default.
- Benchmarks belong under `benches/` or `xtask`.
- Adversarial checks must stay in `just test-adversarial` and cover injection,
  masking, authorization, and OIDC claim-rejection paths without billed APIs.

Coverage is gated through Codecov with an 82% workspace target and 80% patch
target. A failed phase-exit command must be recorded as blocker evidence; do not
claim a checkpoint complete until the blocker is fixed or explicitly carried in
the ultragoal evidence.
