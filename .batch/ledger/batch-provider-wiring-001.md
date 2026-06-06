---
batch: batch-provider-wiring-001
items: provider:yahoo
outcome: done
---

# batch-provider-wiring-001 — wire tdw-provider-yahoo

Closes the last `unwired-http` provider: all 31 HTTP-capable provider crates
are now wired into `default_registry()` behind feature gates. The
provider-wiring bucket's remainder is the two `needs-design` items
(`fileset`, `ws`).

## Scope

11-line diff, exactly the established pattern (cf. #140/#141):
- `crates/tdw-service-api/Cargo.toml` — optional dep, `provider-yahoo`
  feature key, `all-http-providers` membership.
- `crates/tdw-service-api/src/lib.rs` — cfg-gated
  `YahooHttpEquityHistoricalFetcher` import + `registry_entry()` registration
  + offline-default test cfg list updated.
- No changes needed in `tdw-provider-yahoo` (its fetcher already matched the
  #141 Fetcher pattern).

## Gate evidence

| Gate | Command | Result |
| --- | --- | --- |
| fmt | `cargo fmt --all -- --check` | pass |
| clippy | `cargo clean -p tdw-service-api && cargo clippy --workspace --all-targets -- -D warnings` | pass |
| udf-wasm combo | `cargo clippy -p tdw-service-api --features udf-wasm --all-targets -- -D warnings` | pass |
| yahoo combo | `cargo clippy -p tdw-service-api --features provider-yahoo --all-targets -- -D warnings` | pass |
| tests | `cargo test --workspace` | pass (0 failed; `default_registry_is_offline_only` keeps the offline default at 3 providers) |
| clean-room | `cargo run -p xtask -- clean-room-audit` | pass |

## PR

(link added on creation)
