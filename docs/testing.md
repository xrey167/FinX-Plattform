# Testing

## Local Gates

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p xtask -- clean-room-audit
```

## Test Tiers

- Unit tests live beside implementation code in `mod tests`.
- Integration tests belong under `tests/integration/`.
- Property tests belong under `tests/property/`.
- End-to-end tests belong under `tests/e2e/` and should avoid billed APIs by default.
- Benchmarks belong under `benches/` or `xtask`.
- Adversarial tests belong under `tests/adversarial/` and must document their safety
  assumptions.

The initial coverage floor is intentionally bootstrapped at zero until real feature
code exists. The workflow files are already shaped so the floor can ratchet up per
phase.
