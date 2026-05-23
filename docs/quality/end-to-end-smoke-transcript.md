# End-to-End Smoke — Captured Transcript (G009)

Concrete demonstration that the smoke composition produces deterministic
output across both binaries. Captured on 2026-05-23 against
`work/g009-end-to-end-smoke` HEAD.

## Integration test

```
$ cargo test -p tdw-test-utils --all-targets
running 3 tests
test tests::container_specs_cover_minimal_profile ... ok
test tests::ohlcv_fixture_is_deterministic ... ok
test smoke::tests::smoke_reports_roundtrip_for_fileset_aapl ... ok

test result: ok. 3 passed; 0 failed; 0 ignored

     Running tests\end_to_end_smoke.rs
running 2 tests
test end_to_end_smoke_drives_runtime_provider_and_storage ... ok
test end_to_end_smoke_normalizes_query_symbol ... ok

test result: ok. 2 passed; 0 failed; 0 ignored
```

5/5 green.

## `tdw-service` (JSON output)

```
$ cargo run -p tdw-service -- AAPL
{
  "provider": "fileset",
  "endpoint": "equity_historical",
  "query_symbol": "AAPL",
  "rows_fetched": 2,
  "blob_key": "smoke/AAPL.json",
  "blob_bytes_written": 285,
  "blob_bytes_read": 285,
  "roundtrip_ok": true,
  "storage_root": "C:\\Users\\…\\Temp\\tdw-service-<pid>-<nanos>-<seq>"
}
```

## `tdw-cli` (one-line summary)

```
$ cargo run -p tdw-cli -- MSFT
tdw-cli provider=fileset endpoint=equity_historical symbol=MSFT rows=2 blob=smoke/MSFT.json bytes=285 roundtrip=true
```

The CLI demonstrates symbol normalization (`MSFT` passed through unchanged here;
the lowercase " msft " case is exercised by
`end_to_end_smoke_normalizes_query_symbol`).

## Local toolchain note

Local verification ran under `RUSTUP_TOOLCHAIN=stable` because the workstation's
pinned `1.95.0` toolchain is wedged by a corrupted `rustc_driver-*.dll` and a
fs-lock from a parallel session that blocks `rustup toolchain uninstall`.
CI runs in clean containers against the pinned `1.95.0` (see
`.github/workflows/ci.yml`), which is the authoritative gate.
