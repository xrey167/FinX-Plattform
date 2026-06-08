# tdw-test-utils architecture

`tdw-test-utils` provides three things: deterministic data fixtures, integration
container specs, and the offline end-to-end smoke. It is offline and constant by
design so it can be the "still works" floor for CI and the `--smoke` path in the
service binaries.

## Module map

| Path | Contents |
| --- | --- |
| `src/lib.rs` | `fixtures` and `containers` modules. |
| `src/smoke.rs` | `SmokeReport`, `run_end_to_end_smoke`, `allocate_storage_root`. |

## Key items

### `fixtures` module

Constant, deterministic constructors over `tdw-domain` types:

- `ohlcv(symbol) -> Vec<EquityHistoricalData>` — two fixed OHLCV bars.
- `instrument(symbol) -> Instrument` — `{ symbol, name, venue: "XNAS" }`.
- `research_note(id) -> ResearchNote` — a fixed synthetic note.

Determinism is a contract: `ohlcv("AAPL") == ohlcv("AAPL")`.

### `containers` module

`ContainerSpec { name, image, default_port }` plus `const fn` constructors:
`postgres` (5432), `clickhouse` (8123), `qdrant` (6333), `meilisearch` (7700),
`minio` (9000), `redis` (6379). These are metadata only — nothing is launched.

### `smoke` module

`SmokeReport` is the structured outcome (provider, endpoint, query symbol, rows,
blob key, bytes written/read, `roundtrip_ok`, storage root). It is serialized to
JSON by the binaries and asserted field-by-field by integration tests.

`run_end_to_end_smoke(symbol, storage_root)`:

1. `tdw_service_api::fetch_equity_historical("fileset", symbol)` (offline).
2. Serialize the `OBBject` to JSON bytes.
3. `tdw_storage_fs::LocalBlobEngine` `put_object` then `get_object`.
4. Assert a byte-exact roundtrip and populate `SmokeReport`.

`allocate_storage_root(prefix)` returns a unique temp dir
(`{prefix}-{pid}-{nanos}-{seq}`) so parallel smokes never collide.

## Runtime flow (the smoke)

```text
caller / `--smoke`
   └─▶ run_end_to_end_smoke(symbol, root)
          tdw_service_api::fetch_equity_historical("fileset", symbol)
             └─▶ CommandRunner ─▶ FilesetEquityHistoricalFetcher ─▶ fixture rows
          serde_json (OBBject -> bytes)
          LocalBlobEngine.put_object  ─▶  LocalBlobEngine.get_object
          assert bytes match ─▶ SmokeReport { roundtrip_ok: true, .. }
```

## Security posture

Offline and deterministic — no network, no Docker, no secrets. The smoke writes
only into a per-process temp directory and reads it straight back. The
feature flags (`integration`/`property`/`e2e`) gate *consumers'* heavier tiers
but never enable network behavior in this crate.

## Integration points

- `tdw-domain` — fixture data types.
- `tdw-service-api` — the smoke's fetch entrypoint.
- `tdw-storage-fs` — the smoke's local blob engine.
- `tdw-service` / `tdw-cli` — call `run_end_to_end_smoke` from their `--smoke`
  flag and print the `SmokeReport`.
