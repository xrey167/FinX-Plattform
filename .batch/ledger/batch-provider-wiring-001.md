---
batch: batch-provider-wiring-001
items: provider:yahoo
outcome: done
---

# batch-provider-wiring-001 — provider:yahoo

Wire `tdw-provider-yahoo`'s LIVE HTTP fetcher (`YahooHttpEquityHistoricalFetcher`)
into `tdw-service-api` behind a new, off-by-default feature `provider-yahoo-http`,
mirroring the existing `provider-binance-http` feature-gate pattern.

## Changes
- `crates/tdw-service-api/Cargo.toml`: added `provider-yahoo-http = ["tdw-provider-yahoo/http"]`
  next to `provider-binance-http`, and added `"provider-yahoo-http"` to the
  `all-http-providers` aggregate.
- `crates/tdw-service-api/src/dispatcher.rs`: cfg-paired the yahoo import
  (offline `YahooEquityHistoricalFetcher` under `not(provider-yahoo-http)`,
  live `YahooHttpEquityHistoricalFetcher` under `provider-yahoo-http`) and the
  `"yahoo"` ingest dispatch arm.
- `crates/tdw-service-api/src/lib.rs`: cfg-gated the live import and paired the
  `"yahoo"` arm in `fetch_equity_historical`. The offline fetcher's
  `registry_entry()` registration (same `("yahoo","equity_historical")` key)
  is unchanged — only the dispatched fetcher instance differs.

Default features do NOT enable `provider-yahoo-http`, so the offline default
registry (3 providers) is preserved.

## Gate commands + results

### cargo clean -p tdw-provider-yahoo -p tdw-service-api
`Removed 5705 files, 1.8GiB total` — cache busted.

### cargo fmt -p tdw-provider-yahoo -- --check ; cargo fmt -p tdw-service-api -- --check
PASS — `FMT_OK` (no diffs). Used per-crate form per the Windows-206 fallback.

### cargo clippy --workspace --all-targets -- -D warnings
PASS (exit 0).
```
    Checking tdw-service-api v0.1.0 (...\crates\tdw-service-api)
    ...
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 39.88s
```

### cargo test --workspace
PASS (exit 0). No failures.

### cargo run -p xtask -- clean-room-audit
PASS (exit 0).
```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 9.71s
     Running `...\xtask.exe clean-room-audit`
clean-room audit passed
```

### cargo clippy -p tdw-service-api --features provider-yahoo-http --all-targets -- -D warnings
PASS (exit 0).
```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 18.77s
```

### cargo build -p tdw-service-api --features all-http-providers
PASS (exit 0).
```
   Compiling tdw-service-api v0.1.0 (...\crates\tdw-service-api)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 28.31s
```

### cargo clippy -p tdw-provider-yahoo -p tdw-service-api --all-targets -- -W clippy::pedantic -W clippy::nursery
PASS (no NEW warnings introduced on touched lines). The pre-existing baseline
warnings (tdw-agent / tdw-eval-runner / tdw-knowledge / tdw-mask /
tdw-service-api app_state.rs + dispatcher.rs:111/150 fn-level + lib.rs:203
`default_registry` too-many-lines) are all unrelated to the yahoo wiring; none
land on the added import/arm lines.

## Notes
- `YahooHttpEquityHistoricalFetcher` implements `Default` and the same
  `Fetcher<EquityHistoricalQuery, EquityHistoricalData>` trait as the offline
  fetcher, so the dispatch call is identical except for the instance
  (`&YahooHttpEquityHistoricalFetcher::default()`).
- The live HTTP path is additionally env-gated by `TDW_YAHOO_LIVE=1` inside the
  provider crate, so enabling the feature does not cause CI to hit Yahoo.

## PR
https://github.com/xrey167/FinX-Plattform/pull/256
