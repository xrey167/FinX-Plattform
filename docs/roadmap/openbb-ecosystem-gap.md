# OpenBB *ecosystem* parity — gap-map & cutover scoreboard

> **Scope.** This is the **ecosystem** gap-map: the capability *surfaces* the
> OpenBB organization ships across its whole GitHub org — **beyond the Platform's
> data/command surface** (which is tracked separately in
> [`openbb-gap-matrix.md`](openbb-gap-matrix.md), now at **284 catalog routes**).
> It tracks the `openbb-ecosystem-p1` campaign (waves **G001–G010**): the
> non-data capability surfaces — computed analytics (option pricing, forecasting),
> a provider scaffolder, a desktop chart-render host, a Workspace control-plane
> MCP, a chat-bot, and an Excel/Office add-in.
>
> **Clean-room rule (applies to every row):** built from public docs / public API
> shapes only — OpenBB's public documentation and each upstream library's own
> public docs. **Never read or copy OpenBB source code.** Capability and field
> shape are replicated natively against FinX's own crates.

## Cutover status — 2026-06-14 (G010, ecosystem cutover, v1.8.0 prep)

**The campaign is complete.** Every capability surface identified at the start of
`openbb-ecosystem-p1` is now **BUILT** or a **documented deferral**. The ten
waves landed as PRs **#444–#453** (G002 shares G001's number range; see the
per-row merge references). All Rust surfaces are off-by-default-feature-gated
where they pull heavy/native dependencies, so the default build is unaffected.

| Wave | Capability surface | OpenBB-org equivalent | FinX delivery | Crate / package | Status | Merge |
|---|---|---|---|---|---|---|
| **G001** | Option pricing & greeks | (Platform has no computed pricer) | Pure-Rust Black-Scholes, greeks, implied-vol solve, binomial (CRR), Monte-Carlo | `tdw-quant-options` (compute routes) | **DONE** | #444 |
| **G002** | Provider scaffolder | OpenBB provider-extension cookiecutter | `xtask new-provider` — scaffolds a `tdw-provider-<name>` crate against `tdw_core::Fetcher` | `xtask new-provider` | **DONE** | #445 |
| **G003** | Classical forecasting | `openbb-forecast` (statistical models) | Pure-Rust naive/seasonal-naive/RWD, Holt-Winters/ETS, Theta, MSTL, lag-linregr + RMSE/MAE/MAPE/SMAPE backtests + quantile-band anomaly | `tdw-analytics-forecast` (`forecast/*` routes) | **DONE** | #446 |
| **G004** | Workspace SDK completeness | `openbb-platform-pro` Workspace backend + copilot SDK | Widgets/apps backend + copilot bridge brought to SDK-completeness (citations, agent contract) | `tdw-openbb-agent` / `tdw-widgets` | **DONE** | #447 |
| **G005** | Desktop chart-render host | PyWry (Plotly desktop host) | Pure-Rust Plotly host-page assembler + optional native window | `tdw-chart-host` (`gui` feature → `wry`/`tao`) | **DONE** | #448 |
| **G006** | Workspace control-plane MCP | OpenBB Workspace control MCP | Dashboard/widget CRUD + layout + navigate over a catalog-validated `WorkspaceState` → `apps.json` | `tdw-workspace-mcp` | **DONE** | #450 |
| **G007** | Chat-bot surface | `openbb-bot` (Discord/Telegram) | Pure-Rust offline bot core (parser + router + `BotResponse` + chart payload) with feature-gated transports | `tdw-bot` (`discord`/`telegram`/`charts` features) | **DONE** | #451 |
| **G008** | AutoARIMA | `openbb-forecast` AutoARIMA | Pure-Rust `ARIMA(p,d,q)` (Hannan-Rissanen) + Hyndman-Khandakar-style `AutoARIMA` | `tdw-analytics-forecast` (`forecast/arima`, `forecast/autoarima`) | **DONE** | #452 |
| **G009** | Excel / Office add-in | OpenBB Add-in for Excel | Office.js custom functions (`FINX.GET` / `FINX.BYOD` / `FINX.ROUTES`) over the FinX REST API | `integrations/excel-addin` (TypeScript package) | **DONE** | #453 |
| **G010** | Ecosystem cutover | — | This doc + release prep + full-workspace release certification | (docs / release) | **DONE** | this PR |

### Deliberate deferrals (documented, not gaps)

- **Deep-learning forecasting** (RNN/LSTM/GRU, NBEATS, NHITS, TCN, TFT,
  Transformer — OpenBB's `darts`/`torch` tail) is **deferred by decision**, not
  built. It is fundamentally at odds with the pure-Rust, zero-heavy-dependency,
  deterministic, offline-compute posture the analytics surface is built on. The
  full rationale and the supported future path (an off-by-default `candle`/`burn`
  feature, or a sidecar) are recorded in
  [`dl-forecasting-deferral.md`](dl-forecasting-deferral.md). The classical
  suite (G003) + ARIMA/AutoARIMA (G008) cover the keyless, deterministic majority
  of OpenBB's forecasting surface.
- **The Excel add-in ships as a separate TypeScript package**
  (`integrations/excel-addin`), **not** a Rust crate — Office.js add-ins are a
  browser/Office-host JavaScript runtime, so this is the deliberate, idiomatic
  non-Rust deliverable of the campaign. It is a thin client over the same REST
  catalog (all real logic is in its unit-tested pure `src/lib`), so it adds no
  Rust workspace surface and no native dependency.

## Where FinX now *exceeds* the OpenBB Platform

The OpenBB Platform is a **data-aggregation** layer: its `derivatives` and
`forecast` routers largely *fetch* provider-served numbers (e.g. an
intrinio/option-vendor IV surface) rather than *compute* them. This campaign adds
**computed analytics the OpenBB Platform does not compute itself**:

- **Computed option pricing & greeks** (G001) — Black-Scholes, the full greek
  set, implied-vol inversion, a binomial tree, and Monte-Carlo, all pure-Rust and
  deterministic. The OpenBB Platform has no equivalent computed pricer; it surfaces
  vendor option data.
- **Computed classical forecasting** (G003) — a full statistical forecaster +
  backtest + accuracy-measure suite computed in-process, deterministically, with
  no provider call.
- **Computed AutoARIMA** (G008) — `ARIMA`/`AutoARIMA` estimated in pure Rust
  (Hannan-Rissanen + stepwise order search), exposed as offline compute routes.

These are **additive, deterministic, offline `Compute` routes** (no provider, no
network, every figure reproducible from its inputs) — a capability layer above the
Platform's fetch-and-standardize surface.

## Definition of done

The `openbb-ecosystem-p1` campaign is **done** when **every capability surface the
OpenBB organization ships (beyond the Platform's data/command surface) is either
BUILT in FinX or recorded as a documented deferral with a decision and a future
path.** As of G010 (2026-06-14) that condition holds:

- **BUILT:** option-pricing (G001), provider-scaffolder (G002), classical
  forecasting (G003), Workspace-SDK completeness (G004), desktop chart-host
  (G005), Workspace control-plane MCP (G006), chat-bot (G007), AutoARIMA (G008),
  Excel/Office add-in (G009).
- **DOCUMENTED DEFERRAL:** deep-learning forecasting (the `torch`/`darts` tail —
  see [`dl-forecasting-deferral.md`](dl-forecasting-deferral.md)).
- **DELIBERATE NON-RUST DELIVERABLE:** the Excel add-in (a TypeScript Office.js
  package, by the nature of the Office runtime).

No ecosystem surface remains unbuilt without a recorded decision. The command/data
parity scoreboard ([`openbb-gap-matrix.md`](openbb-gap-matrix.md), 284 routes) and
this ecosystem gap-map together account for both halves of OpenBB parity —
**data/command parity** and **ecosystem/capability-surface parity** — and both are
now closed (parity-complete on the keyless/free surface; the only data-side
residual is paid-key / no-public-API providers, a business decision).
