---
batch: batch-lint-debt-008
items: lint:clippy::derive_partial_eq_without_eq
outcome: done
---

# batch-lint-debt-008 — derive_partial_eq_without_eq (leaf-crate slice)

Resolves a bounded slice of `clippy::derive_partial_eq_without_eq`: types that
derive `PartialEq` and whose members are all `Eq`, so they can additionally
derive `Eq`. Mechanical via `cargo clippy --fix`; any type with a float
(`f32`/`f64`) or other non-`Eq` member (e.g. `serde_json::Value`) is reverted —
adding `Eq` there fails to compile and is semantically wrong for float-bearing
financial data.

## Method note

The lint only fires on code that is actually compiled, so the per-crate fix runs
with `--all-targets --all-features` (many candidate structs live behind a
feature gate such as `http`, or in `#[cfg(test)]` modules). None of the touched
crates are schema-bearing (tdw-agent/tdw-event/tdw-protocol/tdw-config), so no
schema regeneration is involved.

## Crates touched

3 crates, 3 `Eq` derives added (1 each).

| Crate | Eq added | Type(s) | Reverted (reason) |
| --- | --- | --- | --- |
| tdw-provider-sec | 1 | `SecFiling` (all `String`) | XBRL/value structs left as-is (`f64` fields) |
| tdw-provider-tiingo | 1 | `TiingoNewsArticle` (`u64`/`String`) | historical-price rows left as-is (`f64` fields) |
| tdw-core | 1 | `CompactReport` (`&'static str`/`usize`) | n/a |

### Crates inspected and DROPPED (no clean candidate)

- `tdw-runtime`, `tdw-storage-postgres/-clickhouse/-router`: the lint does
  **not** fire on their private `Query`/`Row` test/example structs (only used as
  generic args), so `clippy --fix` produced no change — dropped, nothing to
  revert.
- `tdw-provider-deribit/-finnhub/-cboe/-eia/-fmp/-velodata/-ecb/-oecd/-fred/-bls/-binance/-geckoterminal/-adanos`
  and `tdw-spatial`, `tdw-llm`, `tdw-alerts`, `tdw-exec`, `tdw-rollout`,
  `tdw-feature-store`, `tdw-knowledge`: candidate structs carry `f32`/`f64`
  fields (financial prices/scores/observations) or transitively hold
  `serde_json::Value` / `EventMsg`, so `Eq` is impossible/wrong — left deriving
  only `PartialEq`. `SymbolMatch`/`CompanyNewsItem` (finnhub), `AlertDirection`
  (alerts) etc. already had `Eq`.

### Scope-creep guard

`clippy --fix` (which runs the full warn-level lint set, not only the requested
`-W`) tried to apply two **unrelated** fixes — `unneeded return`
(`tdw-core/src/lib.rs:310`, feature-gated) and `unused import: AlertDirection`
(`tdw-alerts/src/lib.rs:326`, `--all-features` only). Both were reverted; they
are pre-existing lints outside the `derive_partial_eq_without_eq` family.

## Reverted / not-touched (float or non-Eq member)

Float-bearing structs in the same and other leaf crates were intentionally left
deriving only `PartialEq` (adding `Eq` does not compile / is semantically
wrong): e.g. adanos sentiment (`f64`), spatial `Point`/`BoundingBox` (`f64`),
alerts `PriceAlert` (`f64`), economic-provider observations (`value: f64`),
`EventMsg`/`ReplayFrame` (`f32` + `serde_json::Value`).

## Gate evidence

(filled per crate below)

### tdw-provider-sec (`SecFiling` lives behind the `http` feature)

| Gate | Command | Result |
| --- | --- | --- |
| fmt | `cargo fmt -p tdw-provider-sec -- --check` | pass |
| clippy | `cargo clean -p tdw-provider-sec; cargo clippy -p tdw-provider-sec --all-targets --all-features -- -D warnings` | pass (0 warnings) |
| tests | `cargo test -p tdw-provider-sec --all-features` | pass (5 passed, 0 failed) |

### tdw-provider-tiingo (`TiingoNewsArticle` lives behind the `http` feature)

| Gate | Command | Result |
| --- | --- | --- |
| fmt | `cargo fmt -p tdw-provider-tiingo -- --check` | pass |
| clippy | `cargo clean -p tdw-provider-tiingo; cargo clippy -p tdw-provider-tiingo --all-targets --all-features -- -D warnings` | pass (0 warnings) |
| tests | `cargo test -p tdw-provider-tiingo --all-features` | pass (15 passed, 0 failed) |

### tdw-core (`CompactReport` compiles under default features)

| Gate | Command | Result |
| --- | --- | --- |
| fmt | `cargo fmt -p tdw-core -- --check` | pass |
| clippy | `cargo clean -p tdw-core; cargo clippy -p tdw-core --all-targets -- -D warnings` | pass (0 warnings) |
| tests | `cargo test -p tdw-core` | pass (46 passed, 0 failed) |

### Workspace

| Gate | Command | Result |
| --- | --- | --- |
| clean-room | `cargo run -p xtask -- clean-room-audit` | pass |
| residual scan | `cargo clean; cargo clippy --workspace --all-targets --all-features -- -W clippy::derive_partial_eq_without_eq` | 0 `derive_partial_eq_without_eq` warnings remaining |

## PR

(link added on creation)
