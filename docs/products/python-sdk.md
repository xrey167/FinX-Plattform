# Python SDK (`finx-platform`) — quickstart

A thin, **generated** Python HTTP client for the daemon's catalog REST API. It
gives OpenBB-Python-package ergonomics — `finx.equity.price.historical(...)` —
over a locally running daemon, returning a `FinXObject` you can convert to a
pandas / polars dataframe or a plain dict.

> This is **not** PyO3 and **not** a re-implementation: the daemon is the
> product, and policy + credentials stay server-side. The SDK only builds
> `GET /api/v1/<route>` requests and wraps the standardized result envelope. The
> package lives at [`sdk/python`](../../sdk/python) and its namespace modules are
> generated from the endpoint catalog (drift-gated by `pysdk-check`).

## Install

The runtime has **zero** third-party dependencies (standard-library `urllib`
only), so the base install pulls nothing transitive. Dataframe conversion is
opt-in via extras.

```bash
pip install finx-platform              # runtime only (stdlib)
pip install finx-platform[pandas]      # + .to_dataframe()
pip install finx-platform[polars]      # + .to_polars()
```

From a checkout (editable install):

```bash
pip install -e sdk/python
pip install -e "sdk/python[pandas]"
```

Python 3.10+ is required.

## Start a daemon with REST enabled

The SDK targets the daemon's catalog REST surface (`/api/v1`), which is off by
default. Build with the `rest-api-route` feature and bind the listener:

```bash
# Loopback REST surface on :7879 (the SDK's default base URL).
TDW_DAEMON_REST_BIND=127.0.0.1:7879 \
  cargo run -p tdw-backend --features rest-api-route
```

See [`rest-api.md`](./rest-api.md) for the full REST surface, status codes, and
auth posture.

## Configuration

| Setting  | Source                                  | Default                 |
|----------|-----------------------------------------|-------------------------|
| Base URL | `FinX(base_url=...)` → `FINX_BASE_URL`   | `http://127.0.0.1:7879` |
| Timeout  | `FinX(timeout=seconds)`                  | `30.0`                  |

```bash
export FINX_BASE_URL=http://127.0.0.1:7879
```

## First call

```python
from finx_platform import FinX

finx = FinX()  # base_url from FINX_BASE_URL, else the loopback default

obj = finx.equity.price.historical(symbol="AAPL", provider="fileset")

obj.to_dict()        # the raw envelope: id / results / provider / warnings / extra
obj.results          # the standardized record list
obj.provider         # the provider key that served the request
obj.warnings         # non-fatal advisories (e.g. provider_fallback)

df = obj.to_dataframe()   # pandas (extra); to_polars() for polars
```

## Call surface

Each router namespace is an attribute on `FinX`; each route is a method named by
its final slash segment, with intermediate segments as nested accessors:

```python
finx.equity.price.historical(symbol="AAPL")        # equity/price/historical
finx.economy.cpi(provider="fred")                  # economy/cpi
finx.fixedincome.government.treasury_rates.t_10y()  # fixedincome/government/treasury_rates/10y
```

Naming rule: route segments are lowercase `[a-z0-9_]`, mapped to Python
identifiers verbatim. A segment that starts with a digit (the maturity tenors
like `10y`, `90d`, `3m`) or is a Python keyword is prefixed with `t_` so it is a
valid identifier — e.g. `…/treasury_rates/10y` → `.treasury_rates.t_10y(...)`.

### Parameters

* **Standardized query params** (`start_date`, `end_date`, `interval`, `period`,
  `limit`) are typed keyword arguments derived from the route's params schema.
* **`provider=`** selects one provider explicitly. Omit it to use the catalog's
  fallback order (offline/keyless fixtures first), accumulating a
  `provider_fallback` warning on each retryable miss:

  ```python
  obj = finx.equity.price.historical(symbol="AAPL")  # no provider -> fallback
  print(obj.provider, obj.warnings)
  ```

* **Provider-specific arguments** (e.g. `symbol`) are passed through `**kwargs`
  and threaded onto the query string.
* **`chart=True`** (on chartable routes) requests a server-rendered chart; it
  surfaces under `obj.extra` / `obj.chart`:

  ```python
  obj = finx.equity.price.historical(symbol="AAPL", provider="fileset", chart=True)
  spec = obj.chart  # None unless the server attached a chart payload
  ```

### Compute routes

`technical/*` indicators are compute routes. The REST surface serves data
(`Fetch`) routes only, so calling a compute method raises `NotImplementedError`
pointing at the MCP tool surface / the daemon `Op` compute path:

```python
finx.technical.rsi(length=14)   # raises NotImplementedError (use MCP / Op)
```

## Errors

| HTTP   | Exception      | Meaning                                        |
|--------|----------------|------------------------------------------------|
| `400`  | `ValueError`   | Unknown route or invalid query parameters.     |
| `502`  | `RuntimeError` | Every candidate provider failed.               |
| other  | `RuntimeError` | Any other non-success response.                |

The server's error message is carried in the exception text.

## Testing the client offline

`Client` takes an injectable `opener` (the `urlopen(request, timeout=...)`
shape), so you can exercise URL building, kwarg→query mapping, and envelope
conversion without a network. The bundled suite uses stdlib `unittest` (no
pytest):

```bash
python3 -m unittest discover sdk/python/tests
```

## Regeneration & drift gate

The namespace modules (`finx_platform/<namespace>.py` and `__init__.py`) are
**generated** from the endpoint catalog and checked in. Only `_client.py`,
`pyproject.toml`, `README.md`, and the tests are hand-written.

```bash
cargo run -p xtask -- pysdk-sync     # regenerate from the catalog
cargo run -p xtask -- pysdk-check    # fail if the checked-in tree drifted
```

CI runs `pysdk-sync` + a `git diff` check, `pysdk-check`, and the Python
`unittest` suite next to the OpenAPI drift gate (see `.github/workflows/ci.yml`).
The generator is deterministic (every map is sorted), so regenerating twice
yields byte-identical output.
