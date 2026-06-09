# tdw-test-utils

Shared test scaffolding for the TDW workspace: deterministic data fixtures,
container specs for integration profiles, and the offline end-to-end smoke that
the `tdw-service` and `tdw-cli` binaries run via `--smoke`.

Although it is a `*-test-utils` crate, it is a normal (non-dev) dependency of the
service binaries because they expose `--smoke` in production builds. Everything
here is `#![forbid(unsafe_code)]` and offline: fixtures are constant, the smoke
uses the `fileset` provider and a local-disk blob engine, and the container specs
are just metadata (no Docker is launched).

## Binaries produced

None. Library crate consumed by binaries and tests.

## Feature flags

| Feature | Purpose |
| --- | --- |
| `integration` | Marker enabling integration-tier tests in consumers. |
| `property` | Marker enabling property-test tiers. |
| `e2e` | Marker enabling end-to-end-tier tests. |

These are gating markers; the crate's own code is feature-agnostic. See the
workspace test policy for how the tiers are used.

## Key environment variables

None. Integration consumers gate live backends on `TDW_*_TEST_URL` variables
(documented in [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md)); this crate
itself reads no environment.

## Quickstart (library)

Use a fixture and run the offline smoke:

```rust,ignore
use tdw_test_utils::fixtures;
use tdw_test_utils::smoke::{allocate_storage_root, run_end_to_end_smoke};

let bars = fixtures::ohlcv("AAPL");          // deterministic 2-row OHLCV
assert_eq!(bars.len(), 2);

let root = allocate_storage_root("my-smoke"); // unique temp dir
let report = run_end_to_end_smoke("AAPL", root).await?;
assert!(report.roundtrip_ok);
# Ok::<(), tdw_core::Error>(())
```

The smoke composes `tdw_service_api::fetch_equity_historical` → JSON →
`tdw_storage_fs::LocalBlobEngine` put/get and asserts a byte-exact roundtrip.

See [`examples/basic.rs`](examples/basic.rs):
`cargo run -p tdw-test-utils --example tdw_test_utils_basic`.

## See also

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — fixtures, container specs, smoke path.
- `tdw-service` / `tdw-cli` — the `--smoke` entrypoints.
