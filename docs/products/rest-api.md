# REST API (catalog-derived) — quickstart

The daemon exposes an optional, **catalog-derived** REST surface: every `Fetch`
route in the endpoint catalog (`tdw-endpoint-catalog`) becomes a `GET` endpoint
under `/api/v1/`, plus a generated OpenAPI 3.1 document at `/openapi.json`. The
surface follows the OpenBB "routes are data, not code" model — there is no
per-route handler; routes are resolved from the catalog at request time and
dispatched through the **same policy-guarded `Op::FetchData` path** the daemon's
other ingress (`POST /op`) uses, returning the standardized `ResultEnvelope`.

> Built behind the `rest-api-route` feature (which implies `transport-http`).
> Off by default; enable it and bind the listener via `TDW_DAEMON_REST_BIND`.

## URL surface

| Method | Path                          | Description                                   |
|--------|-------------------------------|-----------------------------------------------|
| `GET`  | `/api/v1/{route...}?<params>` | Resolve a catalog route and fetch its records |
| `GET`  | `/openapi.json`               | The generated OpenAPI 3.1 document            |

`{route...}` is a slash-namespaced catalog route, e.g.
`equity/price/historical`, `fixedincome/government/treasury_auctions`,
`economy/cpi`. Query-string parameters are parsed into the route's standardized
query params (numbers/bools are coerced to JSON scalars; everything else stays a
string). An explicit `provider=` selects exactly one candidate and never falls
back; with no `provider`, the catalog's registered candidates are tried in
declaration order (offline/keyless fixtures first), accumulating a
`provider_fallback` warning on each retryable miss.

### Status codes

| Code  | Meaning                                                              |
|-------|---------------------------------------------------------------------|
| `200` | Success — body is the `ResultEnvelope` (`id`/`results`/`provider`/`warnings`/`extra`). |
| `400` | Unknown catalog route (body lists the known routes) **or** invalid query parameters. |
| `404` | Path is not under `/api/v1/` (and is not `/openapi.json`).          |
| `405` | Method other than `GET` on this family.                             |
| `502` | Every candidate provider failed (provider-side error).              |

## Auth

The REST family is **unauthenticated at the transport**, mirroring the daemon's
primary `POST /op` ingress (which performs no per-request signature check). The
security boundary is the loopback bind plus the in-handler policy guard
(`enforce_request_path_with_backend`), which runs before any catalog resolution.
The safe default is a loopback bind (`127.0.0.1`). Operators who bind a
non-loopback address **must** front the daemon with a token/mTLS/reverse-proxy
layer and wire an auth-backed policy (`TDW_OIDC_*`), exactly as for the TCP
transport (see `docs/release/data-backend-runbook.md`).

## Start the daemon with REST enabled

Build `tdw-backend` (or `tdw-service`) with the `rest-api-route` feature and set
`TDW_DAEMON_REST_BIND` to bind the REST listener:

```bash
# Local run (loopback REST surface on :7879).
TDW_DAEMON_REST_BIND=127.0.0.1:7879 \
  cargo run -p tdw-backend --features rest-api-route
```

Via the compose stack (`docker-compose.yaml` / `tdw-backend`), set
`TDW_DAEMON_REST_BIND` in the service environment (loopback inside the network,
fronted by your ingress). The listener logs on startup:

```
tdw-backend: REST listener on http://127.0.0.1:7879 (/api/v1/<route> /openapi.json)
```

## curl examples

Happy path — historical OHLCV for a symbol (offline `fileset` fixture):

```bash
curl 'http://127.0.0.1:7879/api/v1/equity/price/historical?symbol=AAPL&provider=fileset'
```

```json
{
  "id": "equity/price/historical",
  "results": [
    { "symbol": "AAPL", "date": "2026-05-20", "open": 100.0, "high": 102.0, "low": 99.0, "close": 101.0, "volume": 10000 }
  ],
  "provider": "fileset",
  "warnings": [],
  "extra": { "route": "equity/price/historical", "arguments": { "provider": "fileset" } }
}
```

Provider fallback — omit `provider=` to try the catalog candidates in order; a
retryable miss records a `provider_fallback` warning and advances:

```bash
curl 'http://127.0.0.1:7879/api/v1/equity/price/historical?symbol=AAPL'
```

Unknown route — `400` with the known-routes list:

```bash
curl -i 'http://127.0.0.1:7879/api/v1/does/not/exist?symbol=AAPL'
# HTTP/1.1 400 Bad Request
# { "error": "unknown catalog route: does/not/exist; known: commodity/..., equity/price/historical, ..." }
```

Invalid parameter — `400` (a validation error fails fast, no fallback):

```bash
curl -i 'http://127.0.0.1:7879/api/v1/equity/price/historical?symbol=AAPL&interval=bogus'
# HTTP/1.1 400 Bad Request
```

## Charts (`chart=true`)

Append `chart=true` to any **chartable** route and the envelope gains a `chart`
field carrying a renderable chart spec. The spec is a **Plotly figure** — the
JSON object `{ "data": [...traces], "layout": {...} }` that the
[plotly.js](https://plotly.com/javascript/) library renders directly in the
browser. It is built server-side with plain JSON (no native graphics
dependency); the client does the rendering. When `chart=true` is absent the
`chart` field is omitted entirely, so existing payloads are unchanged.

The shape is detected from the route's rows: OHLCV rows (e.g.
`equity/price/historical`) yield a `candlestick` trace (plus a `volume` bar
subplot when volume is present); a single-value `date`+`value` series (e.g. the
fixed-income rate and yield-curve routes) yields a `scatter` line; a
`technical/*` indicator over a fetched price series yields the indicator line(s)
overlaid on a candlestick of the source bars. A chartable route whose rows are
neither shape attaches no spec and records a `chart_unsupported` warning.

```bash
curl 'http://127.0.0.1:7879/api/v1/equity/price/historical?symbol=AAPL&provider=fileset&chart=true'
```

```json
{
  "id": "equity/price/historical",
  "results": [ { "symbol": "AAPL", "date": "2026-05-20", "open": 100.0, "high": 102.0, "low": 99.0, "close": 101.0, "volume": 10000 } ],
  "provider": "fileset",
  "warnings": [],
  "extra": { "route": "equity/price/historical", "arguments": { "provider": "fileset" } },
  "chart": {
    "data": [
      { "type": "candlestick", "name": "OHLC", "x": ["2026-05-20"], "open": [100.0], "high": [102.0], "low": [99.0], "close": [101.0] },
      { "type": "bar", "name": "Volume", "x": ["2026-05-20"], "y": [10000.0], "yaxis": "y2" }
    ],
    "layout": { "title": "Candlestick", "template": "plotly_dark", "showlegend": true }
  }
}
```

To render it, hand `envelope.chart` straight to `Plotly.newPlot(div,
envelope.chart.data, envelope.chart.layout)`.

The OpenAPI document:

```bash
curl 'http://127.0.0.1:7879/openapi.json' | jq '.openapi, (.paths | length)'
# "3.1.0"
# 60
```

## OpenAPI generation (no drift)

The document is **generated**, never hand-written. `cargo run -p xtask --
openapi-sync` assembles it from `tdw_endpoint_catalog::catalog()` — each `Fetch`
route → a `GET` path under `/api/v1/<route>`, query parameters from the route's
`params_schema`, and the route's model schema in `components.schemas` (deduped by
title) — and writes the deterministic, pretty JSON to
[`docs/schemas/openapi.json`](../schemas/openapi.json). The server embeds that
same file via `include_str!`, so the served doc and the checked-in doc cannot
diverge. CI runs `openapi-sync` + a `git diff` check and `openapi-check` as a
drift gate next to `catalog-check`.

## Credentials

Per-provider credentials are documented in the `tdw-config` credential registry
(`tdw_config::credential_registry()` / `resolve_credential(provider)`): a
provider key maps to its environment variable (and an optional
`user_settings.json`-style config-file key). FRED (`FRED_API_KEY`) and EIA
(`EIA_API_KEY`) are wired today; the offline/keyless routes (`fileset`, `yahoo`,
`sec`, `government_us`, `federal_reserve`) need no credential.
