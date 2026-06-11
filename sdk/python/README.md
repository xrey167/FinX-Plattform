# finx-platform — Python SDK

A thin, **generated** HTTP client for the FinX Platform daemon's catalog REST
API. It gives OpenBB-style call ergonomics over a locally running daemon:

```python
from finx_platform import FinX

finx = FinX()  # base_url from FINX_BASE_URL, else the loopback default
obj = finx.equity.price.historical(symbol="AAPL", provider="yahoo")

obj.to_dict()        # always available, zero dependencies
obj.to_dataframe()   # pandas (extra)
obj.to_polars()      # polars (extra)
```

This is **not** a re-implementation of the platform — it is a generated HTTP
client. The daemon is the product; policy and credentials stay server-side. The
SDK only builds `GET /api/v1/<route>` requests and wraps the standardized result
envelope.

## Install

```bash
pip install finx-platform              # runtime: standard library only
pip install finx-platform[pandas]      # + to_dataframe()
pip install finx-platform[polars]      # + to_polars()
```

From a checkout (editable):

```bash
pip install -e sdk/python
pip install -e "sdk/python[pandas]"
```

## Configuration

| Setting    | Source                                   | Default                  |
|------------|------------------------------------------|--------------------------|
| Base URL   | `FinX(base_url=...)` → `FINX_BASE_URL`    | `http://127.0.0.1:7879`  |
| Timeout    | `FinX(timeout=seconds)`                   | `30.0`                   |

The default base URL is the daemon's default REST bind. Start the daemon with
the REST surface enabled (`TDW_DAEMON_REST_BIND`, `rest-api-route` feature) — see
`docs/products/rest-api.md`.

## Call surface

Each router namespace is an attribute on `FinX`, and each route is a method whose
final slash segment is the method name:

```python
finx.equity.price.historical(symbol="AAPL")        # equity/price/historical
finx.economy.cpi(provider="fred")                  # economy/cpi
finx.fixedincome.government.treasury_rates()       # fixedincome/government/treasury_rates
```

* Standardized query parameters (`start_date`, `end_date`, `interval`, `period`,
  `limit`) are typed keyword arguments.
* `provider=` selects one provider explicitly; omit it to use the catalog's
  fallback order (offline/keyless fixtures first).
* Provider-specific arguments (e.g. `symbol`) are passed through `**kwargs`.
* Chartable routes accept `chart=True` to request a server-rendered chart, which
  appears under `obj.extra` / `obj.chart`.

### Provider fallback

```python
# No provider -> try the catalog candidates in order; a retryable miss records a
# `provider_fallback` warning and advances. Inspect obj.warnings / obj.provider.
obj = finx.equity.price.historical(symbol="AAPL")
print(obj.provider, obj.warnings)
```

### Compute routes

`technical/*` indicators are compute routes; the REST surface serves data
(`Fetch`) routes only. Calling a compute method raises `NotImplementedError`
pointing at the MCP tool surface / the daemon `Op` compute path.

## Errors

| HTTP   | Exception        |
|--------|------------------|
| `400`  | `ValueError`     |
| `502`  | `RuntimeError`   |
| other  | `RuntimeError`   |

The server's error message is carried in the exception text.

## Regeneration

The namespace modules (`finx_platform/<namespace>.py` and `__init__.py`) are
generated from the endpoint catalog and checked in. Do not edit them by hand:

```bash
cargo run -p xtask -- pysdk-sync     # regenerate
cargo run -p xtask -- pysdk-check    # drift gate (CI)
```

Only `_client.py`, `pyproject.toml`, `README.md`, and the tests are
hand-written.
