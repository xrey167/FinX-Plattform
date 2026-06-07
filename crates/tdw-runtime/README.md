# tdw-runtime

The provider execution runtime. `CommandRunner` owns a `tdw-core`
`ProviderRegistry` and `Credentials`, and drives a `Fetcher` through its
`fetch` (terminal) and `run_streaming` (progress-wrapped) paths. It is the thin
seam between "which providers exist" and "run this one now", used by
`tdw-service-api` for both the one-shot fetch and the ingest paths.

Pure orchestration: `#![forbid(unsafe_code)]`, no network of its own — it calls
the `Fetcher` you hand it. The streaming wrapper turns a single terminal fetch
into a deterministic `start → done` progress stream so the daemon's streaming
contract holds even for non-streaming providers.

## Binaries produced

None. Library crate.

## Feature flags

None.

## Key environment variables

None directly. Credentials are passed programmatically via
`CommandRunner::with_credentials`; provider endpoint/secret env vars are owned by
the individual `tdw-provider-*` crates and documented in
[`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).

## Quickstart (library)

Register a provider and run a fetcher (offline `fileset` fetcher shown):

```rust,ignore
use tdw_runtime::CommandRunner;
use tdw_core::RegistryEntry;
use serde_json::json;

let mut registry = tdw_core::ProviderRegistry::default();
registry.register(RegistryEntry::fetcher("fileset", "equity_historical"))?;
let runner = CommandRunner::new(registry);

// `fetcher` is any `impl Fetcher<Q, D>` (e.g. FilesetEquityHistoricalFetcher).
let object = runner.run(&fetcher, json!({ "symbol": "AAPL" })).await?;
assert_eq!(object.provider, "fileset");
# Ok::<(), tdw_core::Error>(())
```

`run_streaming` returns a `ProgressStream<D>` emitting two `Progress` frames and
a terminal `Done(OBBject)`.

See [`examples/basic.rs`](examples/basic.rs):
`cargo run -p tdw-runtime --example basic`.

## See also

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — runner internals and the streaming shim.
- `tdw-service-api` — the daemon consumer of `CommandRunner`.
