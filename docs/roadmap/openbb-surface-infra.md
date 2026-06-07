# OpenBB Platform — Infrastructure & Platform Capability Surface

> **Header note:** Derived from public OpenBB documentation (docs.openbb.co, openbb.co
> feature/blog pages, PyPI package descriptions) for clean-room gap analysis.
> **No OpenBB source code was consulted.** This maps the *infrastructure & platform*
> surface only — data commands are explicitly out of scope.

This document surveys everything OpenBB Platform offers that is **not** a data command:
the provider integration framework, the REST API server, the CLI, charting, settings &
credentials, the extension/plugin architecture, the standardized data model (`OBBject`),
caching/export, SDK ergonomics, the MCP server, and AI/copilot features.

---

## 1. Architecture Overview (the platform spine)

OpenBB Platform is built around **`openbb-core`**, a Python package built on **FastAPI +
Pydantic** that acts as the foundation REST API. Core responsibilities (docs-level):

- Receives a request, identifies the appropriate **data provider**, queries it, and
  returns results in standardized models.
- Exposes a `@router.command` decorator (extends `FastAPI().add_api_route()` with extra
  params) so any command auto-becomes a REST endpoint.

Layered request flow:

1. **Routers** (e.g. `openbb-equity`, `openbb-economy`) — the intermediate layer defining
   endpoints like `equity/profile`, `equity/price/historical`. **100+ endpoints** exist
   across nested routers.
2. **Providers** — outermost layer that does data extraction via a **TET / ETL pipeline**
   (Transform–Extract–Transform): validate params & apply defaults → fetch from external
   source → standardize/structure the response.

**Key design principle:** data acquisition is decoupled from visualization. The Python
library, OpenBB Workspace, and the Excel Add-in are all **API clients** of the same
`openbb-core`, so every client gets identical data. The **Python library is dynamically
generated from the OpenAPI spec** — calling a Python function issues the same query as a
direct HTTP request.

---

## 2. Provider Integration (the data-source connector framework)

Provider extensions are **independent data-source connectors** — each can be installed or
removed without disrupting core. Each provider uses a **fetcher model** so retrievals run
independently of routers/interfaces. A provider maps its source to the platform's
**standard models** (standard query params + standard data schema), enabling
multi-provider interchangeability on a single endpoint (e.g. `equity/price/historical`
accepts `provider=` from several sources).

### 2a. Public / built-in providers (install with `pip install openbb`)

| Provider package | Source | Credential / API key |
|---|---|---|
| `openbb-bls` | Bureau of Labor Statistics | Yes (`bls_api_key`) |
| `openbb-congress-gov` | U.S. Congress | Yes (`congress_gov_api_key`) |
| `openbb-cftc` | Commodity Futures Trading Commission | Yes (`cftc_app_token`) |
| `openbb-ecb` | European Central Bank | No |
| `openbb-imf` | International Monetary Fund | No |
| `openbb-federal-reserve` | U.S. Federal Reserve | No |
| `openbb-fred` | FRED (St. Louis Fed) | Yes (`fred_api_key`) |
| `openbb-government-us` | U.S. Government Data | No |
| `openbb-oecd` | OECD | No |
| `openbb-polygon` | Polygon.io | Yes (`polygon_api_key`) |
| `openbb-sec` | SEC EDGAR | No |
| `openbb-us-eia` | U.S. Energy Information Admin | Free (registration) |

### 2b. Third-party providers (install with `pip install "openbb[all]"`)

Not installed by default; **endpoint & data availability varies by provider and
subscription tier.**

| Extension | Source | Tier | Credential field |
|---|---|---|---|
| `openbb-alpha-vantage` | Alpha Vantage | Free | `alpha_vantage_api_key` |
| `openbb-benzinga` | Benzinga | Paid | `benzinga_api_key` |
| `openbb-biztoc` | Biztoc News | Free | `biztoc_api_key` |
| `openbb-cboe` | Cboe | Free / none | — |
| `openbb-deribit` | Deribit | Free / none | — |
| `openbb-econdb` | EconDB | Free / none | `econdb_api_key` (optional) |
| `openbb-famafrench` | Ken French Data Library | Free / none | — |
| `openbb-finra` | FINRA | Free / none | — |
| `openbb-finviz` | Finviz | Free / none | — |
| `openbb-fmp` | Financial Modeling Prep | Free tier | `fmp_api_key` |
| `openbb-intrinio` | Intrinio | Paid | `intrinio_api_key` |
| `openbb-nasdaq` | Nasdaq Data Link | Free tier | `nasdaq_api_key` |
| `openbb-seeking-alpha` | Seeking Alpha | Free / none | — |
| `openbb-tmx` | TMX (Canada) | Free / none | — |
| `openbb-tradier` | Tradier | Free / none | `tradier_api_key` + `tradier_account_type` |
| `openbb-tiingo` | Tiingo | Free tier | `tiingo_token` |
| `openbb-tradingeconomics` | TradingEconomics | Paid | `tradingeconomics_api_key` |
| `openbb-yfinance` | Yahoo Finance | Free / none | — (no key) |

**Takeaways for gap analysis:**
- A large share of providers are **free/no-key** (yfinance, Cboe, FINRA, Finviz, ECB, IMF,
  Fed, OECD, SEC, etc.), giving a usable platform with zero credentials.
- Provider design is **pluggable per-endpoint**: the same standard model can be served by
  many providers, with `provider=` choosing the source.
- The full **list grows over time** — OpenBB positions data sources as "public agencies
  with open APIs; some require registration but all are free" for the core set.

---

## 3. REST API Server (FastAPI surface)

OpenBB ships a **ready-to-use FastAPI REST API**. Two documented launch paths:

- Raw uvicorn: `uvicorn openbb_core.api.rest_api:app --host 0.0.0.0 --port 8000 --reload`
- Helper CLI (`openbb-platform-api` package): the **`openbb-api`** command starts a
  FastAPI + Uvicorn dev server.

`openbb-api` flags (docs-level):

| Flag | Purpose |
|---|---|
| `--app` | Absolute path to Python file with the FastAPI instance |
| `--name` | FastAPI instance name (default `app`) |
| `--factory` | App name is a factory function |
| `--editable` | Allow `widgets.json` edits at runtime |
| `--no-build` | Load existing `widgets.json` without update check |
| `--exclude` | JSON list of API paths to exclude |
| `--widgets-json` / `--apps-json` | Paths to Workspace config files |

Defaults: host `127.0.0.1`, port `6900`, auto-fallback to next free port. Remaining args
pass through to `uvicorn.run`.

**How commands become endpoints:** the `@router.command` decorator turns each command into
an auto-generated REST route. Every router/provider you install is *automatically* exposed.

**OpenAPI docs:** interactive API documentation lives at **`/docs`** from the server root
(standard FastAPI Swagger UI), browsable in any HTTP-capable browser.

**Auth story (docs-level):** authorization is **optional for core services** — the API can
run open. User settings / credentials are read when the FastAPI app is "authorized." HTTP
basic-auth style login exists for the dev server; credentials resolve from
`user_settings.json`. (Auth is intentionally lightweight; production hardening is left to
the deployer.)

**Workspace bridge:** the API auto-generates **`widgets.json`** (and serves `apps.json`)
by analyzing endpoint return types & OpenAPI schema, so endpoints become OpenBB Workspace
widgets without separate config. `openapi_extra` dicts customize per-endpoint widget
behavior.

---

## 4. CLI (command tree + interactive shell)

The **OpenBB Platform CLI** (`openbb-cli`, launched via the `openbb` command) is the
successor to the sunset OpenBB Terminal. It is a thin interactive client over the Platform.

- **Command tree concept:** organized into **menus** (asset classes / domains such as
  equity, crypto, economy) that you navigate into, with **commands** inside each menu —
  mirroring the Platform's router hierarchy.
- **Auto-completion engine:** presents choices based on current menu/command; typing shows
  suggestions for commands and menus; pressing space after a command lists its arguments.
  Bounded choice lists are scrollable with up/down arrow keys. Greatly reduces keystrokes
  and the need to open help.
- **Arguments:** passed as `--`flags; `-h` for help; a `provider` argument selects the data
  source per command.
- **Routines:** record and replay multi-command workflows; save to **OpenBB Hub**; share
  via public URL. Supports advanced routines (variables/scripting) to run many commands in
  one go.
- **Export:** results export to spreadsheets/CSV/XLSX/JSON; exports, routines, and other
  user content are stored in the **OpenBBUserData** folder.
- **Settings / login:** integrates with OpenBB Hub login for syncing settings & routines.

---

## 5. Charting

- **Library:** **Plotly** (the `openbb-charting` infrastructure extension provides "Plotly
  charting components").
- **Enable per call:** pass `chart=True` to a supported endpoint
  (e.g. `obb.equity.price.historical(symbol="TSLA", chart=True)`).
- **Customize:** `chart_params` nested dict (e.g. title, technical `indicators` like EMA
  lengths).
- **Return location:** rendered chart lives on the response object's **`chart`** attribute.
- **Methods:** `show()` (display + save to `OBBject`), `to_chart()` (redraw + save, with
  optional data entry point), `table()` (interactive table view).
- **Chart types:** pre-built charts per endpoint, including **candlestick with technical
  indicators**; unsupported endpoints can route data through quantitative-analysis commands
  to auto-generate **line charts**.
- **Display backend:** **PyWry** opens standalone native windows for charts/tables
  (`pip install "openbb-charting[pywry]"`; Linux needs extra webkit deps).
- **Theming:** default theme `dark`; user prefs `chart_style` and `table_style`.

---

## 6. User Settings & Credentials Management

- **Storage:** all settings persist locally in **`~/.openbb_platform/user_settings.json`**
  (created on first run). Read when the Python client initializes or the FastAPI app is
  authorized.
- **Sections:** the JSON holds **`credentials`**, **`preferences`**, and **`defaults`**.

**Credentials** (per-provider keys), examples of the field naming convention:
`fmp_api_key`, `polygon_api_key`, `benzinga_api_key`, `fred_api_key`, `nasdaq_api_key`,
`intrinio_api_key`, `alpha_vantage_api_key`, `biztoc_api_key`, `tradier_api_key`
(+ `tradier_account_type`), `tradingeconomics_api_key`, `tiingo_token`, `bls_api_key`,
`congress_gov_api_key`, `cftc_app_token`.

Setting methods:
- **Python (session):** `obb.user.credentials.intrinio_api_key = "my_api_key"`.
- **Environment variables** and system settings are also supported.
- **OpenBB Hub:** credentials/settings can be stored centrally and synced.

**Preferences** (in `user_settings.json`) documented include: `data_directory`,
`export_directory`, `output_type` (e.g. `"OBBject"` vs dataframe/dict), `metadata`,
`chart_style`, `table_style` (plus HTTP-cache directory location). The user-data directory
is where the Python packages store data such as the **data / HTTP cache**.

---

## 7. Extension / Plugin Architecture (3rd-party extensibility)

OpenBB defines **three extension types**, all distributed as standalone PyPI packages and
registered via **Python entry points** (so installing the package auto-wires it):

1. **Router (command) extensions** — define new endpoints. Use the **`Router`** class and
   the **`@router.command`** decorator (linked to a standard `model=`); routers nest via
   `include_router`. Registered under the **`openbb_core_extension`** entry-point group.
2. **Provider extensions** — add data sources. Built from **`QueryParams`** + **`Data`**
   classes and a **`Fetcher`** implementing **`transform_query` / `extract_data` /
   `transform_data`** (the TET/ETL pattern). The **`ProviderInterface`** maps all installed
   providers to their callables and resolves the `provider=` argument at execution.
   Registered under the **`openbb_provider_extension`** entry-point group.
3. **OBBject (accessor) extensions** — extend the result object with new methods/namespaces
   via the **`@obbject_accessor`** decorator (e.g. add `.charting`, `.technical`).

OpenBB provides a **Cookiecutter template** to scaffold new extensions. After installing an
extension package, its routes/providers are **automatically available** in the Python
package and the REST API (the static Python interface is regenerated from the API surface).

---

## 8. Data Model Standardization — the `OBBject` envelope

Every command returns an **`OBBject`** result envelope with consistent fields:

- **`id`** — UUID tag
- **`results`** — serializable result data (list of standardized data objects)
- **`provider`** — the provider that served the request
- **`warnings`** — list of warnings
- **`chart`** — chart object (populated when `chart=True`)
- **`extra`** — extra metadata dict (includes `arguments`, `route`, `timestamp`)

**Standardization:** results conform to **standard data models** (shared field names/types
across providers) so the same schema is returned regardless of which provider answered.

**Conversion / interop helpers on `OBBject`:** `to_df()` / `to_dataframe()`,
`to_dict()`, `to_polars()`, `to_numpy()`, `to_llm()` (LLM-friendly serialization), and
`show()` (render the attached chart). This makes results immediately consumable as pandas /
polars / numpy / JSON.

---

## 9. Caching & Export

- **HTTP cache:** the user-data directory stores a **data / HTTP cache**; the cache
  directory location is configurable via preferences. (Caching is provider-request level,
  not a separate query-cache service at docs level.)
- **Export:** controlled by the `export_directory` preference; the CLI and clients export
  results to **CSV / XLSX / JSON** and spreadsheets. The **OpenBBUserData** folder is the
  canonical store for exports, routines, styles, and other user content.
- **Output type:** the `output_type` preference selects the default return shape
  (`OBBject`, dataframe, etc.).

---

## 10. Notebooks / SDK Ergonomics

- **Single import surface:** `from openbb import obb` exposes the entire command tree with
  IDE auto-complete (the Python package is generated from the OpenAPI spec).
- **Frictionless start:** works with zero credentials using free providers (e.g. yfinance);
  add keys incrementally.
- **Direct dataframe workflow:** `obb.<...>().to_df()` returns analysis-ready pandas;
  `to_polars()`/`to_numpy()`/`to_dict()` for other targets.
- **Inline charts:** `chart=True` + `.show()` for notebook visualization.
- **Excel Add-in & Workspace** reuse the same API, so notebook work is portable to those
  surfaces.

---

## 11. MCP Server (agentic interface)

The **`openbb-mcp-server`** extension exposes the Platform's REST endpoints to LLM agents
over **Model Context Protocol**. Capabilities (docs-level):

- Converts any FastAPI app into an MCP server; supports **stdio, SSE, and streamable-HTTP**
  transports.
- **Dynamic tool discovery:** discovery tools let an agent browse categories and activate
  only the tools it needs, keeping the initial tool list small to avoid token bloat;
  visibility is **per-session** (each client has its own active toolset; multiple agents
  connect independently).
- **Granular exposure control** via `openapi_extra.mcp_config` (`MCPConfigModel`):
  `expose=False` hides a route; `mcp_type` (`tool` / `resource` / `resource_template`);
  `methods` (which HTTP verbs to expose); `exclude_args` (hide internal params).

---

## 12. AI / Copilot Features (Workspace tier)

These are **OpenBB Workspace** features (the commercial frontend), documented publicly:

- **OpenBB Copilot:** AI assistant embedded in Workspace; natural-language queries,
  multi-turn conversation with history/context, retrieves data from multiple sources,
  performs analysis, generates insights. Has access to the active dashboard's widgets
  (including uploaded files), the OpenBB API, and any custom backend endpoints added to
  Workspace.
- **Workspace MCP / Agents Integration:** lets external MCP-compatible agents (Claude Code,
  Codex, etc.) do "real financial work" inside Workspace, on the user's data, governed by
  design. An **OpenBB AI SDK** and a **Pydantic-AI agent integration** path are documented
  for building custom Workspace agents.
- **MCP Tools in Copilot:** connect third-party data providers / analytical services /
  specialized tools without custom development; Copilot can chain tools sequentially per
  prompt.
- **App Marketplace:** distribution surface for Workspace apps/agents.

---

## Gap-Analysis Summary (infra surface, at a glance)

| Capability area | OpenBB approach (docs-level) |
|---|---|
| Provider framework | ~30 pluggable provider extensions, per-endpoint `provider=`, many free/no-key |
| REST API | Auto-generated FastAPI from `@router.command`; `/docs` OpenAPI; optional auth; Workspace `widgets.json` |
| CLI | Menu/command tree mirroring routers; autocomplete; routines (Hub-shareable); CSV/XLSX/JSON export |
| Charting | Plotly via `openbb-charting`; `chart=True`/`chart_params`; PyWry windows; candlestick/line |
| Settings & creds | `~/.openbb_platform/user_settings.json` (credentials/preferences/defaults); env vars; Hub sync |
| Extensions | 3 types (router / provider / OBBject) via entry-point groups; Cookiecutter scaffold |
| Data model | `OBBject` envelope (id/results/provider/warnings/chart/extra) + standard models; `to_df/dict/polars/numpy/llm` |
| Caching/export | HTTP cache in user-data dir; `export_directory`; OpenBBUserData folder; `output_type` |
| SDK ergonomics | `from openbb import obb`, OpenAPI-generated, dataframe-first, zero-key start |
| MCP server | `openbb-mcp-server`; stdio/SSE/HTTP; dynamic tool discovery; per-route exposure config |
| AI/Copilot | Workspace Copilot + Workspace MCP + AI SDK (commercial frontend tier) |

---

### Sources (public OpenBB docs / blog / PyPI — no source code read)

- Providers: https://docs.openbb.co/odp/python/extensions/providers
- API keys & credentials: https://docs.openbb.co/odp/python/settings/user_settings/api_keys
- REST API quickstart: https://docs.openbb.co/python/quickstart/rest_api
- openbb-api interface: https://docs.openbb.co/odp/python/extensions/interface/openbb-api
- Architecture blog: https://openbb.co/blog/exploring-the-architecture-behind-the-openbb-platform/
- Charting extension: https://docs.openbb.co/odp/python/extensions/infrastructure/openbb-charting
- CLI docs / blog: https://docs.openbb.co/cli/ , https://openbb.co/blog/introducing-the-openbb-platform-cli
- OBBject / basic response: https://docs.openbb.co/platform/user_guides/basic_response
- Extension development: https://docs.openbb.co/python/developer/extension_types/provider , .../router
- MCP server: https://docs.openbb.co/odp/python/extensions/interface/openbb-mcp , https://pypi.org/project/openbb-mcp-server/
- Copilot / Workspace AI: https://docs.openbb.co/workspace/openbb-copilot , https://openbb.co/blog/introducing-workspace-mcp/
- Preferences / user data: https://docs.openbb.co/python/settings/user_settings/preferences , https://docs.openbb.co/cli/openbbuserdata
