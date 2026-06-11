# FinX ⇄ OpenBB parity — the surface at a glance

> **Clean-room note:** Every contract below was implemented from **public** OpenBB
> and source-vendor documentation only (`docs.openbb.co`, the OpenBB-AI SDK
> reference, each provider's own public API docs). **No OpenBB source code was
> consulted.** FinX replicates *capability and field shape*, written natively
> against the `tdw_core::Fetcher` / `tdw-service-api` machinery.

This is the single entry point that ties together the OpenBB-parity surface
delivered across phases 1 and 2 (P1 + P2). Everything here is **derived from one
artifact** — the endpoint catalog (`tdw-endpoint-catalog`) — so the REST routes,
the OpenAPI document, the Workspace widgets, the MCP tools, and the warehouse
ingest paths all stay in lock-step with a single typed source of truth.

As of **2026-06-11** (after P2's data-breadth + warehouse waves), the catalog
exposes **119 routes / 91 provider candidates** (`xtask catalog-check` green):
**77 `Fetch` routes** + **`technical/`, `quantitative/`, `econometrics/` `Compute`
routes**. The generated OpenAPI 3.1 document carries **77 paths / 28 schemas**
(`xtask openapi-check` green). P2 added FMP fundamentals completion + discovery +
screener, CFTC Commitments of Traders, a benzinga news cluster, the technical
long-tail indicators, dynamic per-route MCP tools, and warehouse landing tables;
the per-wave detail and the deferred backlog live in the
[gap matrix](../roadmap/openbb-gap-matrix.md).

---

## What FinX offers vs OpenBB

| Capability | OpenBB | FinX equivalent | Pointer |
|---|---|---|---|
| Standardized endpoint catalog | `@router.command` over `openbb-core` | `tdw-endpoint-catalog` — 119 routes, `CatalogEntry { route, kind, params_schema, model, candidates, … }` | [gap matrix](../roadmap/openbb-gap-matrix.md) |
| REST API | auto-generated FastAPI, `/docs` | catalog-derived `GET /api/v1/{route...}` → policy-guarded `Op::FetchData` → `ResultEnvelope` | [rest-api.md](./rest-api.md) |
| OpenAPI spec | FastAPI-generated | programmatic OpenAPI 3.1 at `GET /openapi.json`, checked in + drift-gated (`openapi-sync`/`openapi-check`) | [rest-api.md](./rest-api.md), [`docs/schemas/openapi.json`](../schemas/openapi.json) |
| Provider interchange | `provider=` selects a source | ordered candidates per route; no `provider=` → declaration-order fallback with a `provider_fallback` warning; explicit `provider=` never falls back | [rest-api.md](./rest-api.md) |
| MCP server | `openbb-mcp-server` | `tdw-mcp` Streamable-HTTP, incl. `technical.*` analytics tools + read-only widget-catalog tools (`tdw.widgets.list/describe`, `tdw.apps.list`) | [mcp-quickstart.md](./mcp-quickstart.md) |
| Workspace data backend | `widgets.json` / `apps.json` | `tdw-widgets` serves `GET /widgets.json` (one widget per Fetch route), `GET /apps.json`, `GET /widget-data/{route...}` | [openbb-workspace-backend.md](./openbb-workspace-backend.md) |
| Workspace copilot | `agents.json` + `POST /query` SSE | `tdw-openbb-agent` serves `GET /agents.json` + `POST /v1/query` SSE (openbb-ai vocabulary), stateless two-request widget-data pattern | [openbb-workspace-agent.md](./openbb-workspace-agent.md) |
| Analytics | technical / quant / econometrics routers | 20 `technical/*` `Compute` routes today (quant + econometrics are P2) | [gap matrix](../roadmap/openbb-gap-matrix.md) |
| Warehouse | none | **every catalog route is also warehouse-ingestible** — `Op::FetchData` (fetch-without-persist) and `IngestBatch` (persist) are two modes of the *same* catalog entry | [warehouse-install.md](./warehouse-install.md) |

The key divergences from OpenBB are deliberate and documented in the parity
plan: routes are **data, not per-route handlers**; credentials stay
**server-side** behind the daemon's policy guard (never a client-side
`user_settings.json`); and the widget / agent JSON contracts are **projections**
of the typed catalog, never the internal model.

---

## Quickstart pointers

| To do this | Start here |
|---|---|
| Call data over REST + read the OpenAPI spec | [rest-api.md](./rest-api.md) |
| Drive FinX from an MCP agent (incl. `technical.*` tools) | [mcp-quickstart.md](./mcp-quickstart.md) |
| Add FinX as an OpenBB Workspace **data backend** (`widgets.json`) | [openbb-workspace-backend.md](./openbb-workspace-backend.md) |
| Add FinX as an OpenBB Workspace **copilot** (`agents.json`) | [openbb-workspace-agent.md](./openbb-workspace-agent.md) |
| Learn the whole surface from runnable, offline examples | [examples/workspace](../../examples/workspace/README.md) |
| Install / run the warehouse daemon | [warehouse-install.md](./warehouse-install.md) |

All four surfaces are **off by default** and env-gated; each binds its own
loopback listener on the same `tdw-backend` daemon and shares the daemon's
policy / hook / mask guards.

---

## Environment variables

Each surface is enabled by setting its bind variable (and compiling the matching
feature). The Workspace backend and the copilot **share** the `TDW_WORKSPACE_*`
CORS + API-key posture but bind on **separate** variables.

| Variable | Surface (feature) | Default | Purpose |
|---|---|---|---|
| `TDW_DAEMON_REST_BIND` | REST `/api/v1` + `/openapi.json` (`rest-api-route`) | unset (off) | `host:port` for the catalog REST listener. |
| `TDW_WORKSPACE_BIND` | Workspace backend `/widgets.json` `/apps.json` `/widget-data` (`workspace-route`) | unset (off) | `host:port` for the Workspace data-backend listener. |
| `TDW_AGENT_BIND` | Workspace copilot `/agents.json` `/v1/query` (`agent-route`) | unset (off) | `host:port` for the copilot listener (its **own** bind; the `agents.json` query URL is composed from it). |
| `TDW_WORKSPACE_API_KEY` | workspace backend **and** copilot | unset (no auth) | Shared key; when set, every request must send a matching `X-TDW-API-KEY` header (constant-time). **Fail-closed:** a non-loopback bind without it refuses to start. |
| `TDW_WORKSPACE_CORS_ORIGINS` | workspace backend **and** copilot | `https://pro.openbb.co` + local dev origins | Comma-separated CORS origin allow-list. |
| `TDW_MCP_HTTP_TOKEN` | `tdw-mcp` Streamable-HTTP | unset | Bearer token; mandatory on a non-loopback bind, required on every request when set. |
| `TDW_MCP_ALLOWED_ORIGINS` | `tdw-mcp` Streamable-HTTP | unset (loopback origins only) | Comma-separated extra exact `Origin`s to accept, e.g. `https://pro.openbb.co`. Loopback always allowed. |
| `FRED_API_KEY` | FRED provider (via `tdw-config` registry) | unset | FRED credential; resolved through `tdw_config::resolve_credential("fred")`. |
| `EIA_API_KEY` | EIA provider (via `tdw-config` registry) | unset | EIA credential; resolved through `tdw_config::resolve_credential("us_eia")`. |

Per-provider credentials live in the `tdw-config` credential registry
(`tdw_config::credential_registry()` / `resolve_credential(provider)`), which
maps a provider key to its environment variable (and an optional
`user_settings.json`-style config-file key). FRED and EIA are wired today; the
offline/keyless routes (`fileset`, `yahoo`, `sec`, `government_us`,
`federal_reserve`, `ecb`, `cboe`) need no credential.

---

## Consolidated manual OpenBB Workspace interop checklist

> **CI cannot cover this.** The OpenBB Workspace frontend at `pro.openbb.co` is
> **proprietary**, so automated CI cannot drive it. CI covers the backend
> contracts instead — serde round-trips, derivation/golden snapshots, and HTTP
> integration tests against an in-memory daemon (see the per-surface docs). The
> steps below are the **manual** gate to run against a live Workspace after any
> change to the contracts or the catalog.

### A. Data backend (widgets)

1. **Start the daemon** with `--features workspace-route` and `TDW_WORKSPACE_BIND`
   set (e.g. `127.0.0.1:7900`).
2. In Workspace → Apps / Data, **add a custom backend** pointing at
   `http://127.0.0.1:7900`. If `TDW_WORKSPACE_API_KEY` is set, add header
   `X-TDW-API-KEY: <key>`.
3. **Manifest loads.** Confirm Workspace ingests `widgets.json` without error and
   lists the FinX widgets (e.g. the `equity/price/historical` chart widget).
4. **Widget renders + param sync.** Add the equity historical widget; confirm it
   loads `AAPL` bars offline (the `fileset` fixture, no keys) and re-fetches when
   you change the `symbol` ticker param.
5. **App loads.** Confirm `apps.json` surfaces the "FinX Market Overview" app and
   its tab lays out the configured widgets.

### B. Copilot (agent)

6. **Start the daemon** with `--features agent-route` and `TDW_AGENT_BIND` set
   (e.g. `127.0.0.1:6900`). Confirm `GET http://127.0.0.1:6900/agents.json`
   returns the one-copilot document.
7. **Register the copilot** in Workspace, pointing at `http://127.0.0.1:6900`;
   confirm "FinX Copilot" appears in the copilot picker.
8. **No-widget question** (e.g. "What is a P/E ratio?") streams token-by-token
   (a `reasoning_step` then `message_chunk`s) with no widget fetch.
9. **Grounded question.** Attach the `equity/price/historical` AAPL widget and
   ask about it; confirm the two-request flow (a `get_widget_data` step, then a
   second request streaming the grounded answer ending in a **citation**).

### C. MCP server registration

10. **Bind `tdw-mcp`** with `tdw-mcp --http <host:port>` (default `127.0.0.1:8788`).
    For a network-reachable bind, set `TDW_MCP_HTTP_TOKEN` (the bind otherwise
    refuses to start) and front it with TLS.
11. **Add the origin.** Browser requests from `https://pro.openbb.co` are
    cross-origin, so set `TDW_MCP_ALLOWED_ORIGINS=https://pro.openbb.co` (it is
    **not** allow-listed by default).
12. **Register as an app `mcp_server`** in `apps.json`'s `mcp_servers` so the
    in-app Copilot can call the same tools that back the widgets; confirm the
    read-only widget-catalog tools (`tdw.widgets.list/describe`, `tdw.apps.list`)
    resolve and that each widget's `mcp_tool.tool_id` maps to a real MCP tool
    (the citation contract, enforced by a `tdw-mcp` unit test).

> **Auth check (optional, both surfaces).** With `TDW_WORKSPACE_API_KEY` set,
> confirm a request **without** the `X-TDW-API-KEY` header is rejected `401` and
> the configured key is accepted.
