# OpenBB Workspace Backend (custom data backend)

> **Clean-room note:** The widgets.json / apps.json contract implemented here is
> derived from **public** OpenBB Workspace developer documentation only
> (`docs.openbb.co/workspace`). No OpenBB source code was consulted.

This product surface makes the trading-data-warehouse usable as a **custom data
backend inside OpenBB Workspace** (`pro.openbb.co`). It serves three endpoints
the Workspace expects from a backend, byte-compatible with the published
*backends-for-openbb* contract, all **derived automatically from the endpoint
catalog** — the typed catalog stays the single source of truth.

## What it serves

| Method   | Path                               | Returns                                              |
|----------|------------------------------------|-----------------------------------------------------|
| `GET`    | `/widgets.json`                    | Widget manifest (one widget per catalog Fetch route)|
| `GET`    | `/apps.json`                       | The curated default app (FinX Market Overview)      |
| `GET`    | `/widget-data/{route...}?<params>` | Result rows for a widget, via the daemon fetch path |
| `OPTIONS`| any of the above                   | CORS preflight                                       |

- **Derivation** lives in `crates/tdw-widgets`: `derive_widget(&CatalogEntry)`
  projects each route into a `WidgetConfig` (params from the params schema +
  a synthesized `symbol` ticker param, columns from the model schema), and
  `catalog_widgets()` / `widgets_json()` / `apps_json()` assemble the documents.
  60 Fetch-route widgets are derived; `Compute` routes are excluded for v1
  (follow-up — they carry no provider fetcher).
- **Transport** lives in `crates/tdw-app-server/src/workspace_route.rs`
  (`serve_workspace_http`), a hand-rolled HTTP/1.1 surface (no axum/hyper) that
  mirrors the catalog-derived REST route family. `widget-data` reuses the **same**
  policy-guarded `Op::FetchData` seam (`RestApiHandler` / `RestApiState`) as the
  REST surface, so it never bypasses the daemon's policy / hook / mask guards.

## Contract decisions

- **`dataKey = "results"`.** The daemon returns the `ResultEnvelope` directly,
  whose row array lives under `results`. Each derived widget therefore declares
  `data.dataKey = "results"`, and `/widget-data/...` returns the envelope
  verbatim, so Workspace reads rows from the same key the backend emits.
- **camelCase field names** match the public docs exactly: `gridData`, `minW`,
  `minH`, `paramName`, `headerName`, `cellDataType`, `formatterFn`, `renderFn`,
  `multiSelect`, `columnsDefs`, `showAll`, `refetchInterval`, `staleTime`,
  `chartType`. `mcp_tool` / `mcp_server` / `tool_id` / `mcp_servers` keep their
  documented snake_case spelling.
- **MCP binding.** Every widget binds `mcp_tool = { mcp_server: "tdw-mcp",
  tool_id: "tdw.provider.fetch" }` so an agent in Workspace can fetch the same
  data the widget shows.
- **Widget type.** `chart` when the catalog marks the route `chartable`, else
  `table` (with `columnsDefs` derived from the model schema).

## Setup

The workspace listener is **off by default** and env-gated. Enable it on the
`tdw-backend` daemon by compiling with the `workspace-route` feature and setting
`TDW_WORKSPACE_BIND`:

```sh
cargo run -p tdw-backend --features workspace-route --target-dir target
# with, in the environment:
#   TDW_WORKSPACE_BIND=127.0.0.1:7900
```

Then in OpenBB Workspace, add a custom backend pointing at
`http://127.0.0.1:7900` (or your bound address).

### Environment variables

| Variable                     | Default                         | Purpose                                                              |
|------------------------------|---------------------------------|---------------------------------------------------------------------|
| `TDW_WORKSPACE_BIND`         | unset (listener off)            | `host:port` to bind the workspace surface on.                       |
| `TDW_WORKSPACE_API_KEY`      | unset (no auth)                 | Shared key; when set, every request must send a matching `X-TDW-API-KEY` header (constant-time compare). |
| `TDW_WORKSPACE_CORS_ORIGINS` | `https://pro.openbb.co` + local dev origins | Comma-separated CORS origin allow-list.                 |

**Fail-closed bind.** Mirroring `TDW_MCP_HTTP_TOKEN` semantics, the listener
**refuses to start on a non-loopback bind** (e.g. `0.0.0.0:7900`) unless
`TDW_WORKSPACE_API_KEY` is set. A loopback bind (`127.0.0.1` / `localhost`)
needs no key. This prevents an unauthenticated, network-reachable fetch surface.

### CORS

Workspace runs in a browser, so the family answers `OPTIONS` preflights and
stamps `Access-Control-Allow-Origin` (echoing an allow-listed origin),
`Access-Control-Allow-Methods: GET, OPTIONS`, and
`Access-Control-Allow-Headers: Content-Type, Authorization, X-TDW-API-KEY` on
every response. Configure the allow-list with `TDW_WORKSPACE_CORS_ORIGINS`; the
default permits `https://pro.openbb.co` plus `http://localhost:1420` /
`http://127.0.0.1:1420` (the documented Workspace dev origins).

## Manual interop checklist (Workspace frontend)

The OpenBB Workspace frontend at `pro.openbb.co` is **proprietary**, so automated
CI cannot drive it. After any change to the contract or derivation, run this
manual check against a live Workspace:

1. **Add the backend.** In Workspace → Apps / Data → add a custom backend with
   URL `http://127.0.0.1:7900` (your `TDW_WORKSPACE_BIND`). If
   `TDW_WORKSPACE_API_KEY` is set, add a header `X-TDW-API-KEY: <key>`.
2. **Manifest loads.** Confirm Workspace ingests `widgets.json` without error and
   lists the FinX widgets (e.g. the `equity/price/historical` chart widget).
3. **Widget renders.** Add the equity historical widget to a dashboard; confirm
   it loads rows offline (the `fileset` fixture yields `AAPL` bars without keys)
   and the chart draws.
4. **Param sync.** Change the `symbol` ticker param; confirm the widget re-fetches
   and re-renders for the new symbol.
5. **App loads.** Confirm `apps.json` surfaces the "FinX Market Overview" app and
   its tab lays out the configured widgets.
6. **Citations / agent.** If using Copilot, confirm the `tdw-mcp` server is
   reachable and the widget's `mcp_tool` fetches the same data.

> **Note:** Steps 2–6 exercise the proprietary Workspace frontend and therefore
> **cannot be covered by automated CI**. CI covers the backend contract instead:
> serde round-trips, derivation unit tests, the full `widgets.json` golden
> snapshot (`crates/tdw-widgets/tests/golden_widgets_json.rs`), and HTTP
> integration tests against an in-memory daemon
> (`crates/tdw-service-api/tests/workspace_route_e2e.rs`: manifest parse,
> offline `widget-data` happy path, CORS preflight, auth `401`). The fail-closed
> bind rule is unit-tested in `crates/tdw-backend/src/server/mod.rs`.
