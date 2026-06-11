# 80 — Python SDK over the REST API

The daemon exposes the catalog under a REST surface (`GET /api/v1/<route>`). The
checked-in, **stdlib-only** Python SDK in `sdk/python/finx_platform` gives
OpenBB-style call ergonomics over it — `finx.equity.price.historical(symbol="AAPL")`.
This example is a small script that drives that SDK against a locally booted
daemon.

Unlike the Rust examples, this one does not boot a server itself: it talks to a
daemon you start separately, so the script stays a readable, dependency-free
client.

## What it teaches

- How to point the SDK at a daemon (`base_url` argument, then `FINX_BASE_URL`,
  then the loopback default `http://127.0.0.1:7879`).
- The call shape: namespaces (`finx.equity.price.historical(...)`) returning a
  `FinXObject` whose `.results` are the standardized rows.

## Run (manual, end to end)

1. Start the daemon's REST surface from the workspace root — the listener is
   env-gated on `TDW_DAEMON_REST_BIND` and compiled behind the daemon's
   `rest-api-route` feature:

   ```sh
   TDW_DAEMON_REST_BIND=127.0.0.1:7879 cargo run -p tdw-backend --features rest-api-route --target-dir target
   ```

2. Run the script:

   ```sh
   FINX_BASE_URL=http://127.0.0.1:7879 python examples/workspace/80-python-sdk/main.py
   ```

   It fetches `equity/price/historical` for AAPL from the offline `fileset`
   fixture (no key needed) and prints the first rows.

## CI

This script is stdlib-only and import-clean, so the repo's `py_compile` / pysdk
gate verifies it compiles. The end-to-end run against a live daemon is the manual
step above — there is no in-process Python<->Rust harness, so the run is not
automated.
