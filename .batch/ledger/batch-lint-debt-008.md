---
batch: batch-lint-debt-008
items: lint:clippy::derive_partial_eq_without_eq
outcome: in-progress
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

| Crate | Eq added | Type(s) | Reverted (reason) |
| --- | --- | --- | --- |
| tdw-provider-sec | 1 | `SecFiling` (all `String`) | XBRL/value structs left as-is (`f64` fields) |

## Reverted / not-touched (float or non-Eq member)

Float-bearing structs in the same and other leaf crates were intentionally left
deriving only `PartialEq` (adding `Eq` does not compile / is semantically
wrong): e.g. adanos sentiment (`f64`), spatial `Point`/`BoundingBox` (`f64`),
alerts `PriceAlert` (`f64`), economic-provider observations (`value: f64`),
`EventMsg`/`ReplayFrame` (`f32` + `serde_json::Value`).

## Gate evidence

(filled per crate below)

### tdw-provider-sec

| Gate | Command | Result |
| --- | --- | --- |
| fmt | `cargo fmt -p tdw-provider-sec -- --check` | pass |
| clippy | `cargo clean -p tdw-provider-sec; cargo clippy -p tdw-provider-sec --all-targets --all-features -- -D warnings` | pass (0 warnings) |
| tests | `cargo test -p tdw-provider-sec --all-features` | pass (5 passed, 0 failed) |

## PR

(link added on creation)
