# End-to-End Functional Smoke

**Goal:** G009 — prove the bootstrap composition functions as a working
system end-to-end. This is the **baseline "still works" check** that
every subsequent production tranche (G010+) must keep green.

## What the smoke exercises

The smoke drives a request through the same composition path a production
service request will travel, end-to-end, deterministically and offline:

```text
caller
  └─ tdw_test_utils::smoke::run_end_to_end_smoke
       └─ tdw_service_api::fetch_equity_historical
            └─ tdw_runtime::CommandRunner::run
                 └─ tdw_provider_fileset::FilesetEquityHistoricalFetcher
                      └─ deterministic fixture rows (offline)
  └─ serde_json (OBBject -> bytes)
  └─ tdw_storage_fs::LocalBlobEngine::put_object  (storage write)
  └─ tdw_storage_fs::LocalBlobEngine::get_object  (storage read)
  └─ byte-for-byte roundtrip assertion
  └─ SmokeReport
```

### Participating crates

| Crate | Role in the smoke |
|---|---|
| `tdw-test-utils` | Hosts `smoke::run_end_to_end_smoke` and the integration test |
| `tdw-service-api` | High-level request entry point (`fetch_equity_historical`) |
| `tdw-runtime` | `CommandRunner::run` orchestration |
| `tdw-provider-fileset` | Deterministic equity-historical fetcher |
| `tdw-core` | `OBBject`, `BlobEngine`, `Error`, `Result` traits |
| `tdw-domain` | `EquityHistoricalData` row shape |
| `tdw-storage-fs` | `LocalBlobEngine` write+read |
| `tdw-service` | Binary that invokes the smoke and prints JSON |
| `tdw-cli` | Binary that invokes the smoke and prints a one-line summary |

### Out of scope for G009

Explicitly deferred to later goals:

- **G010** real storage transports (ClickHouse, Postgres, S3, Qdrant, Meilisearch). The smoke uses `tdw-storage-fs` because it is offline-deterministic.
- **G011** real provider HTTP transports. The smoke uses `tdw-provider-fileset` for the same reason.
- **G012** real LLM/embedding transports.
- **G013** durable persistence (Postgres-backed outbox, persistent session/bus/snapshot).
- **G014** release packaging (Dockerfiles, compose, release workflow).
- **G015** policy enforcement binding (auth, sandbox, mask on the request path).
- IPC daemon mode (`tdw-app-server` `<->` `tdw-app-client` over UDS/HttpSSE). The composition pieces exist as types in those crates but no event loop is wired yet; the binaries today invoke the smoke as a programmatic harness, which the G009 spec explicitly allows.

## How to run it

### From the integration test (CI baseline)

```powershell
cargo test -p tdw-test-utils --test end_to_end_smoke
```

Two test functions run:

- `end_to_end_smoke_drives_runtime_provider_and_storage` — full path against symbol `AAPL`.
- `end_to_end_smoke_normalizes_query_symbol` — confirms the fileset symbol-normalization path (`" msft "` -> `MSFT`) flows through the full composition unchanged.

### From the unit test in `smoke` module

```powershell
cargo test -p tdw-test-utils smoke::tests
```

### From the `tdw-service` binary (JSON output)

```powershell
cargo run -p tdw-service -- AAPL
```

Sample output:

```json
{
  "provider": "fileset",
  "endpoint": "equity_historical",
  "query_symbol": "AAPL",
  "rows_fetched": 2,
  "blob_key": "smoke/AAPL.json",
  "blob_bytes_written": 339,
  "blob_bytes_read": 339,
  "roundtrip_ok": true,
  "storage_root": "C:\\Users\\…\\Temp\\tdw-service-<pid>-<nanos>-<seq>"
}
```

A non-default symbol can be supplied as the first positional argument
(it must satisfy the fileset symbol-normalization rules: ASCII
alphanumerics plus `.`, `-`, `_`).

### From the `tdw-cli` binary (one-line summary)

```powershell
cargo run -p tdw-cli -- AAPL
```

Sample output:

```
tdw-cli provider=fileset endpoint=equity_historical symbol=AAPL rows=2 blob=smoke/AAPL.json bytes=339 roundtrip=true
```

## The `SmokeReport` contract

```rust
pub struct SmokeReport {
    pub provider: String,         // "fileset"
    pub endpoint: String,         // "equity_historical"
    pub query_symbol: String,     // symbol after fileset normalization
    pub rows_fetched: usize,      // > 0 on success
    pub blob_key: String,         // "smoke/<symbol>.json"
    pub blob_bytes_written: usize,
    pub blob_bytes_read: usize,
    pub roundtrip_ok: bool,       // true iff the read bytes == written bytes
    pub storage_root: String,     // per-process scratch directory
}
```

Any subsequent tranche that breaks one of these invariants breaks G009.

## Failure modes worth knowing

The smoke is intentionally narrow, but if it ever fails the failure
points to a specific seam:

| Failure | Likely cause |
|---|---|
| Provider error from `fetch_equity_historical` | Fileset fetcher contract regressed; check `tdw-provider-fileset` |
| Serialization error | `OBBject<EquityHistoricalData>` lost a `Serialize`/`Deserialize` derive |
| `put_object` error | `tdw-storage-fs` path resolution or fs permissions regressed |
| `get_object` error | Same as above, or write didn't actually land |
| `roundtrip_ok == false` | Storage layer mutated bytes — should never happen for fs |

## Updating the smoke

The smoke is **deliberately narrow** so it stays fast and deterministic.
Resist the urge to grow it; instead:

- **Add new participant crates** to the composition path when their
  production transports land (G010+), keeping the offline default.
- **Add new symbols / queries** as separate `#[tokio::test]` cases that
  call `run_end_to_end_smoke` with different inputs.
- **Add new storage backends** by parameterizing the `BlobEngine` impl
  (currently hard-coded to `LocalBlobEngine`).

## Cross-references

- Implementation: `crates/tdw-test-utils/src/smoke.rs`
- Integration tests: `crates/tdw-test-utils/tests/end_to_end_smoke.rs`
- Binary entrypoints: `crates/tdw-service/src/main.rs`, `crates/tdw-cli/src/main.rs`
- Ultragoal spec: `.omx/ultragoal/goals.json` (`G009-end-to-end-functional-smoke`)
- Per-crate readiness worksheets receive a `## Smoke Evidence (G009)` note for every crate listed in **Participating crates** above.
