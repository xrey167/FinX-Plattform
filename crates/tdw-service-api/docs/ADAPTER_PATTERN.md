# Adapter Registration Pattern

This document describes the repeatable 3-step recipe for adding a new engine,
provider, or UDF runtime to the TDW daemon. Every adapter added in P5 and
beyond follows this pattern.

---

## Step 1 — Implement the trait in the adapter crate

Create (or extend) the adapter crate under `crates/`. Implement the relevant
core trait:

| Integration point | Trait | Example crate |
|---|---|---|
| Blob storage | `tdw_core::BlobEngine` | `tdw-storage-fs` |
| OLAP engine | `tdw_core::OlapEngine` | `tdw-storage-clickhouse` |
| Relational engine | `tdw_core::RelationalEngine` | `tdw-storage-postgres` |
| Vector engine | `tdw_core::VectorEngine` | `tdw-storage-qdrant` |
| UDF runtime | `tdw_sandbox::SandboxRuntime` seam | `tdw-udf-wasm` |
| Data provider | `tdw_core::Fetcher` / `Streamer` | `tdw-provider-fileset` |

### Example — `tdw-storage-fs` implementing `BlobEngine`

```rust
// crates/tdw-storage-fs/src/lib.rs
#[async_trait::async_trait]
impl tdw_core::BlobEngine for LocalBlobEngine {
    async fn put_object(&self, key: &str, body: Bytes, _content_type: &str) -> Result<()> { … }
    async fn get_object(&self, key: &str) -> Result<Bytes> { … }
}
```

### Example — `tdw-udf-wasm` providing wasm execution

The UDF seam lives in `tdw-sandbox`. The adapter crate exposes a struct with an
`execute(&self, wasm_bytes, func, arg) -> Result<String>` method. The sandbox
crate calls it under the `udf-wasm` feature (see Step 2).

---

## Step 2 — Feature-gate the live path

### In the adapter crate's `Cargo.toml`

No special features needed unless the adapter itself has optional heavy deps
(e.g. a real `wasmi` backend). Add them there if so.

### In `tdw-service-api/Cargo.toml`

Add the adapter as an **optional** dependency and declare a feature:

```toml
[features]
storage-fs = ["dep:tdw-storage-fs"]   # selects LocalFsBlobEngine
udf-wasm   = ["tdw-sandbox/udf-wasm"] # forwards to sandbox crate

[dependencies]
tdw-storage-fs = { workspace = true, optional = true }
```

### In the sandbox crate (for UDF runtimes)

```toml
# crates/tdw-sandbox/Cargo.toml
[features]
udf-wasm = ["dep:tdw-udf-wasm"]

[dependencies]
tdw-udf-wasm = { path = "../tdw-udf-wasm", optional = true }
```

The in-memory / built-in variant remains the **default** when no feature is
selected. CI checks both paths.

---

## Step 3 — Register in `AppState::from_config` (engines) or via existing routing (providers / UDF runtimes)

### Engines — branch inside `select_blob_engine` (or analogous helper)

```rust
// crates/tdw-service-api/src/app_state.rs
fn select_blob_engine(config: &TdwConfig) -> Arc<dyn BlobEngine> {
    #[cfg(feature = "storage-fs")]
    if config.profile == "service" {
        return Arc::new(LocalBlobEngine::new(&config.paths.data_dir));
    }
    Arc::new(InMemoryS3BlobEngine::default())
}
```

The config trigger can be `profile`, a dedicated `blob_backend` field added to
`TdwConfig`, or an environment variable — whatever is cleanest for the adapter.

### Providers — register in `default_registry()`

```rust
// crates/tdw-service-api/src/lib.rs  (default_registry fn)
registry.register_fetcher::<MyFetcher, MyQuery, MyRow>()?;
```

### UDF runtimes — route inside `tdw-sandbox`

```rust
// crates/tdw-sandbox/src/lib.rs
#[cfg(feature = "udf-wasm")]
if request.runtime == UdfRuntime::Wasm {
    return run_wasm(&request);
}
```

---

## What is deliberately deferred

- **Live-network providers** (Polygon, FRED, Alpaca, Binance): their adapter
  crates exist as stubs. Wiring them requires secret-gated integration tests and
  is tracked as a P6/P7 follow-up.
- **Full WASM coverage via `wasmi`**: the current `tdw-udf-wasm` ships a
  deterministic fixture interpreter. A real `wasmi`-backed implementation
  follows the identical Step 1–3 pattern — only the body of `execute()` changes.
- **Feature-flag combinations**: CI currently validates default + all-features
  builds. Matrix testing of individual feature subsets is a P8 quality-gate item.
