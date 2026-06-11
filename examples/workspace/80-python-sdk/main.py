#!/usr/bin/env python3
"""Example 80 — drive the REST API via the checked-in finx_platform Python SDK.

The daemon exposes the catalog under a REST surface (``GET /api/v1/<route>``).
The checked-in, stdlib-only SDK in ``sdk/python/finx_platform`` gives OpenBB-style
call ergonomics over it — ``finx.equity.price.historical(symbol="AAPL")``. This
script boots *nothing itself*: it talks to a daemon you start separately (or, in
CI, the in-memory daemon a Rust harness boots), so it stays a small, dependency-
free client you can read top to bottom.

Run it (manual, end to end):

1. Start the daemon's REST surface, e.g. from the workspace root (the listener
   is env-gated on ``TDW_DAEMON_REST_BIND`` and compiled behind the daemon's
   ``rest-api-route`` feature)::

       TDW_DAEMON_REST_BIND=127.0.0.1:7879 cargo run -p tdw-backend --features rest-api-route --target-dir target

   (Any bind works; point the SDK at it with ``FINX_BASE_URL``.) The loopback
   bind ``http://127.0.0.1:7879`` is what the SDK falls back to.

2. Run this script::

       FINX_BASE_URL=http://127.0.0.1:7879 python examples/workspace/80-python-sdk/main.py

   It fetches ``equity/price/historical`` for AAPL from the offline ``fileset``
   fixture (no provider key needed) and prints the first rows.

CI: this file is import-clean and stdlib-only, so the repo's ``pysdk-check`` /
``py_compile`` gate verifies it compiles. The end-to-end run against a live
daemon is the manual step above.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

# Make the checked-in SDK importable without installing it: add the repo's
# `sdk/python` to the import path. From this file
# (`examples/workspace/80-python-sdk/main.py`) the repo root is three parents up.
_REPO_ROOT = Path(__file__).resolve().parents[3]
_SDK_PATH = _REPO_ROOT / "sdk" / "python"
if str(_SDK_PATH) not in sys.path:
    sys.path.insert(0, str(_SDK_PATH))

from finx_platform import FinX  # noqa: E402  (import after sys.path setup)


def fetch_history(finx: FinX, symbol: str) -> list[dict[str, object]]:
    """Fetch end-of-day history for ``symbol`` from the offline fileset fixture.

    Returns the standardized result rows (possibly empty).
    """
    result = finx.equity.price.historical(symbol=symbol, provider="fileset")
    return result.results


def main() -> int:
    """Drive the SDK against a locally booted daemon and print a summary."""
    base_url = os.environ.get("FINX_BASE_URL")
    finx = FinX(base_url)
    symbol = "AAPL"
    try:
        rows = fetch_history(finx, symbol)
    except (ValueError, RuntimeError, OSError) as exc:
        # The daemon is not running, or the route/query was rejected. Print a
        # readable hint rather than a traceback; see the module docstring.
        print(f"could not reach the daemon REST surface: {exc}", file=sys.stderr)
        print(
            "start it first (see this file's docstring) and set FINX_BASE_URL.",
            file=sys.stderr,
        )
        return 1

    print(f"equity/price/historical for {symbol}: {len(rows)} rows")
    for row in rows[:5]:
        print(f"  {row}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
