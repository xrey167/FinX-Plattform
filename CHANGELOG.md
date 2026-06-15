# Changelog

All notable changes to FinX-Plattform are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
SemVer tags `vMAJOR.MINOR.PATCH` as defined in [`docs/release.md`](docs/release.md).

From `v1.0.0` onward the project follows standard SemVer: `MAJOR` for
backward-incompatible protocol/persistence/API/operator-contract changes,
`MINOR` for backward-compatible user-visible additions, and `PATCH` for
compatible fixes, docs, CI-only changes, and packaging repairs. The pre-1.0
history below used `MINOR` for any user-visible change while the major version
was `0`. The workspace `Cargo.toml` `version` field is intentionally not bumped
per release — releases are tag-driven (see [`docs/release.md`](docs/release.md)).

## [Unreleased]

## [1.8.0] - 2026-06-14

The **OpenBB ecosystem-parity release.** v1.4.0–v1.6.0 reached total OpenBB
command/data parity and v1.7.x shipped the FinX Partner; v1.8.0 closes the
**ecosystem** half — the capability *surfaces* the OpenBB organization ships
beyond the Platform's data/command surface. The `openbb-ecosystem-p1` campaign
(waves G001–G010, PRs #444–#453) added computed analytics, four new operator/
client surfaces, and a provider scaffolder. MINOR — every change is a
backward-compatible **addition**; the native `gui`/`discord`/`telegram`/`charts`
features are **off by default**, so default builds are unaffected. No protocol,
persistence-schema, or operator-contract breaks. See
[`docs/release/v1.8.0-notes.md`](docs/release/v1.8.0-notes.md) and the ecosystem
gap-map [`docs/roadmap/openbb-ecosystem-gap.md`](docs/roadmap/openbb-ecosystem-gap.md).

### Added

- **Computed option pricing & greeks (#444, G001):** a new pure-Rust
  `tdw-quant-options` crate — Black-Scholes, the full greek set, implied-vol
  inversion, a binomial (CRR) tree, and Monte-Carlo — exposed as offline
  `Compute` routes. OpenBB's Platform fetches vendor option data; FinX *computes*
  it.
- **Provider scaffolder (#445, G002):** `xtask new-provider` scaffolds a new
  `tdw-provider-<name>` crate against `tdw_core::Fetcher`, the FinX equivalent of
  OpenBB's provider-extension cookiecutter.
- **Classical forecasting (#446, G003):** a new pure-Rust `tdw-analytics-forecast`
  crate — naive / seasonal-naive / random-walk-drift baselines, Holt-Winters / ETS,
  Theta, MSTL decomposition, lag-feature linear regression, an expanding-window
  backtest harness, RMSE / MAE / MAPE / SMAPE measures, and a quantile-band anomaly
  scan — as `forecast/*` `Compute` routes.
- **Workspace-SDK completeness (#447, G004):** the OpenBB Workspace widgets/apps
  backend (`tdw-widgets`) and copilot bridge (`tdw-openbb-agent`) brought to
  SDK-completeness (citations, the agent contract).
- **Desktop chart-render host (#448, G005):** a new `tdw-chart-host` crate — a
  pure-Rust Plotly host-page assembler with an optional native window behind the
  off-by-default `gui` feature (`wry`/`tao`); the PyWry-equivalent.
- **Workspace control-plane MCP (#450, G006):** a new `tdw-workspace-mcp` crate —
  dashboard/widget CRUD + layout + navigate over a catalog-validated
  `WorkspaceState` that serializes to `apps.json`.
- **Discord/Telegram chat-bot surface (#451, G007):** a new `tdw-bot` crate — a
  pure-Rust offline bot core (parser + router + `BotResponse` + chart payload)
  with feature-gated `discord`/`telegram` transports and a `charts` PNG path
  (all off by default); the `openbb-bot`-equivalent.
- **AutoARIMA (#452, G008):** `ARIMA(p, d, q)` (Hannan-Rissanen two-stage least
  squares) and `AutoARIMA` (Hyndman-Khandakar-style stepwise order search) added
  to `tdw-analytics-forecast`, exposed as the `forecast/arima` and
  `forecast/autoarima` `Compute` routes. Pure Rust, deterministic — OpenBB's
  Platform does not compute these.
- **Excel / Office add-in (#453, G009):** `integrations/excel-addin`, a TypeScript
  Office.js package providing the `FINX.GET` / `FINX.BYOD` / `FINX.ROUTES` custom
  functions over the FinX REST catalog (all real logic in a unit-tested pure
  `src/lib`). Shipped as a separate TS package — the deliberate non-Rust
  deliverable of the campaign.

### Deferred (documented decisions)

- **Deep-learning forecasting** (RNN/LSTM/GRU, NBEATS, NHITS, TCN, TFT,
  Transformer — OpenBB's `torch`/`darts` tail) is **deferred by decision**: it is
  at odds with the pure-Rust, zero-heavy-dependency, deterministic, offline-compute
  posture of the analytics surface. The rationale and the supported future path (an
  off-by-default `candle`/`burn` feature, or a sidecar) are in
  [`docs/roadmap/dl-forecasting-deferral.md`](docs/roadmap/dl-forecasting-deferral.md).

### Also included

- **FinX Partner integrity patch (v1.7.1):** the v1.7.1 patch (released
  2026-06-14) is carried forward — see [1.7.1] below.

## [1.7.1] - 2026-06-14

**FinX Partner integrity patch.** Closes the integrity gaps a post-release
`tdw-partner` re-review found: the learning loop's behavior-shaping was wired to
nothing live, contradicting the docstrings. PATCH — all changes are
backward-compatible (the additions to `TurnOutcome` and the `tdw.partner.undo`
plan are new fields; no breaks). See
[`docs/release/v1.7.1-notes.md`](docs/release/v1.7.1-notes.md).

### Fixed

- **Learned-route reshaping now fires on a live turn (HIGH, W4.2/W4.3):**
  `PartnerCore` gains a gated `InferEngine` seam (`with_infer_engine`); a turn now
  reads the installed (promoted-past-B9) rule set via `routing_hints_from`,
  derives a route-preference list (`route_preferences_from_hints`), and threads it
  onto the `LearningState` snapshot before resolution — so a learned preference
  re-orders the resolved routes on a real turn, not only in a unit test. The
  preference is always the gated signal, never caller-supplied, preserving the
  audit-only/gated posture. A live-turn test asserts the preferred route leads.
- **Walk-forward usefulness harness is a public eval entry point (HIGH, W4.4):**
  `walk_forward::walk_forward_usefulness` is promoted out of `#[cfg(test)]` to a
  public function behind the new `eval-harness` feature, so the gated eval loop /
  a benchmark gate can compute the "more useful with use" metric rather than it
  living only inside a test (the offline `tdw-eval-runner` dependency stays out of
  the default leaf build).
- **Undo respects `AwaitingHuman` (MEDIUM):** `tdw.partner.undo` now surfaces the
  action's `status` and a `needs_confirmation` flag; an action the escalation
  predicate flagged `AwaitingHuman` (high-impact / low-confidence / irreversible)
  no longer yields a ready-to-execute reversal — the human-in-the-loop posture
  survives into the reversal surface.
- **Model error surfaced on the turn outcome (MEDIUM):** `TurnOutcome` gains
  `model_error: Option<String>`; a `complete_streaming` failure is now detectable
  by a write-back caller (it previously had to infer failure from a `Reasoning`
  event string), so a partial answer is never persisted as complete.
- **Ticker stop-words expanded (LOW):** `TICKER_STOP_WORDS` now covers common
  macro/geo/role acronyms (`US`, `EU`, `CPI`, `GDP`, `ETF`, …) so a question like
  "What is US CPI doing?" does not mine `US` as a symbol on an equity route.

## [1.7.0] - 2026-06-14

The **FinX Partner release.** Ties the data, warehouse, knowledge, and learning
layers into one autonomous, learning, human-in-the-loop partner (6 waves, PRs
#437–#442). A new `tdw-partner` crate provides one shared `PartnerCore`, exposed
as thin adapters on MCP, the OpenBB Workspace copilot, and the CLI. MINOR — all
additions, no breaks. See [`docs/release/v1.7.0-notes.md`](docs/release/v1.7.0-notes.md)
and the design spec [`docs/products/finx-partner.md`](docs/products/finx-partner.md).

### Added

- **Partner Core — one conversational front door (#438, W2):** the `tdw-partner`
  crate with `PartnerCore::turn` (resolve → param-extract → fetch → ground →
  write-back), exposed as `tdw.partner.ask` (MCP), the Workspace copilot (the
  `agent_bridge` now routes through `PartnerCore`), and a CLI `ask`. One shared
  core, thin adapters — memory-aware (KG/episodic context per turn), every turn
  written back as episodic memory + candidate findings.
- **Proactive layer — brief + nudges (#439, W3):** a `Nudge` model + brief
  assembler unifying alerts, watchlists, thesis health, open questions,
  contradictions, and staleness; a daily brief scheduled via `tdw-cron` and
  event-driven nudges off the knowledge feed; `tdw.partner.brief`; dismissals
  feed learning.
- **Learning-loop closure (#440, W4):** Partner Core reads the gated runtime
  (`versions()`/adaptivity) per turn so promoted lessons/rules/parameters and
  learned route preferences take effect; trust-dial-filtered retrieval; a
  walk-forward eval harness proving rising usefulness. All behavior changes stay
  B9/eval-gated.
- **Audit & undo surface — audit-only autonomy (#441, W5):** an `audit` feed
  projecting every action with its `why` (over `Proposal.history`,
  `SelfTuneLog`, `LessonAudit`, `tdw.kg.why` — no new store); auto-accept within
  gates with escalation of low-confidence/high-impact/irreversible actions;
  `tdw.partner.audit`/`undo`; `undo` reverses on the governed-forgetting /
  cold-plane machinery; `correct` = undo + feedback that records a `Lesson`.
- **Cohesion + zero-to-partner onboarding (#442, W6):** an anti-duplication test
  enforcing logic-free adapters; a guided first-run workflow; a
  progressive-disclosure manifest leading with `ask`/`brief`/`audit`.
- **Partner design spec (#437, W1):** `docs/products/finx-partner.md` — the
  grounded architecture for the above.

### Fixed

- Partner Core review fixes folded forward each wave: parameterized DataPlane
  fetch (the resolver extracts route + params from the utterance, #439); brief
  dedup-before-sort + bounded dismissal penalty (#440); a real
  learned-preference route re-ordering (prefix-aware) replacing a no-op (#441);
  and **real `undo` reversal per action kind** replacing a silent `Ok` (#442) —
  the linchpin that makes audit-only autonomy genuinely reversible.

## [1.6.0] - 2026-06-14

**Total OpenBB parity.** Closes the OpenBB-parity-total program (G001–G005):
**every OpenBB command with a documented public API — free *or* paid — is now
built**, paid ones key-gated and dormant until a key is configured. The endpoint
catalog grows from **216 → 267 routes** (**169 → 195 `Fetch` routes** + 72
`Compute` routes); OpenAPI paths **169 → 195**. The only residual is scrape /
undocumented-internal sources with no public API (stockgrid, wsj, finviz, multpl)
— a source decision, not an engineering gap. MINOR release — backward-compatible
additions, no breaks. See [`docs/release/v1.6.0-notes.md`](docs/release/v1.6.0-notes.md),
[`docs/products/openbb-total-parity.md`](docs/products/openbb-total-parity.md),
and the TOTAL roll-up in [`docs/roadmap/openbb-gap-matrix.md`](docs/roadmap/openbb-gap-matrix.md).

### Added

- **fixedincome FRED family + economy breadth (G003a, #431):** the remaining FRED
  fixedincome rate/spread/curve family and the economy-router breadth fill →
  standardized `tdw-domain` models, drift-gated.
- **intrinio provider — key-gated (G002, #432):** new `tdw-provider-intrinio`
  (paid `INTRINIO_API_KEY`) wiring intrinio's options unusual/snapshots/IV-surface,
  reported_financials, and forward-P/E routes; code-complete and dormant until a
  paid key is provisioned (no free tier to live-verify).
- **congress.gov + biztoc providers (#430):** the
  `uscongress/{bills,bill_info,bill_text_urls}` cluster (`tdw-provider-congress-gov`,
  free `CONGRESS_GOV_API_KEY`) and biztoc as a second `news/world` candidate
  (`tdw-provider-biztoc`, free `BIZTOC_API_KEY`) — closes the previously
  mislabeled deferrals.
- **compute-router remainder (G003b, #433):** the econometrics / quantitative /
  technical Compute-route remainder → 72 total `Compute` routes (technical 31 /
  quantitative 21 / econometrics 15 / portfolio 5), each also an MCP tool.
- **equity / etf / index / commodity remainder (G003c, #434):** the standardized
  provider-fetch remainder across the equity, etf, index, commodity, regulators
  and index routers, closing the documented-public-API surface.

### Changed

- **Scoreboard truth-up to TOTAL parity (G005, #436):**
  `docs/roadmap/openbb-gap-matrix.md` gains a top **OpenBB-parity TOTAL roll-up**
  section; `docs/products/openbb-parity.md` refreshes the scoreboard to
  **267 catalog / 195 Fetch / 72 Compute** and adds a total-parity statement; new
  `docs/products/openbb-total-parity.md` states the total-parity claim, the
  provider coverage (30+ of 32 built), and the honest residual.

### Fixed

- **scrape-provider assessment + intrinio fixes (G004, #435):** each remaining
  no-API provider (stockgrid, wsj, finviz, multpl) was re-assessed against the
  vendor's own site for a documented public API and confirmed unimplementable
  clean-room (built none); plus intrinio `reported_financials` + Workspace-widget
  fixes folded forward.

## [1.5.0] - 2026-06-14

The **OpenBB command-parity release.** Closes the real OpenBB command-breadth
gap (Phase 4, 11 waves, PRs #419–#428). The endpoint catalog grows from 131 to
**216 routes / 185 provider candidates** — **169 `Fetch` routes (up from 84)** +
47 `Compute` routes — with full parity on the keyless / free-key provider
surface. Remaining OpenBB commands require paid keys or have no public API and
are documented deferrals. MINOR release — backward-compatible additions, no
breaks. See [`docs/release/v1.5.0-notes.md`](docs/release/v1.5.0-notes.md) and
the P4 roll-up in [`docs/roadmap/openbb-gap-matrix.md`](docs/roadmap/openbb-gap-matrix.md).

### Added

- **equity fundamentals breadth (P4W1, #419):** balance/income/cash growth,
  metrics, ratios, dividends, historical EPS/splits, employee count, ESG,
  filings, management + compensation, revenue per segment/geography, earnings
  transcripts (FMP) → standardized `tdw-domain` models.
- **equity discovery/estimates/ownership (P4W2, #420):** search, market
  snapshots, historical market cap, calendar/splits, compare/company_facts
  (SEC), discovery/filings + latest_financial_reports, estimates breadth,
  ownership (insider/institutional/government_trades) (FMP + SEC).
- **yfinance discovery + ETF cluster (P4W3, #421):** four predefined-screen
  discovery routes; etf search/info/historical/sectors/countries/
  equity_exposure/nport_disclosure (FMP + SEC N-PORT + Yahoo).
- **economy breadth (P4W4, #422):** OECD SDMX (CLI, house/share price indices,
  retail prices, GDP forecast, interest rates), Fed/BLS surveys (SLOOS + four
  regional Fed surveys), central-bank holdings, primary-dealer positioning/fails,
  FOMC documents (OECD + FRED + Federal Reserve + BLS + EconDB).
- **fixedincome FRED fill (P4W5, #423):** Svensson yield curve (2y/5y/10y), HQM
  corporate spot (2y/5y/30y), AMERIBOR, EFFR forecast, TCM-EFFR spreads.
- **index/currency/crypto/commodity breadth (P4W6, #424; P4W11, #428):** index
  search/available/constituents/sp500_multiples (Shiller CAPE, NASDAQ),
  currency search/snapshots, crypto search, commodity spot price.
- **derivatives futures + SEC regulator utilities (P4W7+W8, #425):** Deribit
  futures instruments + info; SEC EDGAR symbol_map, filing_headers,
  institutions_search, SIC search, schema_files, RSS litigation.
- **famafrench portfolio returns + imf_utils discovery (P4W9, #427):** Ken
  French breakpoints + US/regional/country portfolio returns + international
  index returns; IMF SDMX dataflow-discovery helpers.
- **FINRA shorts/dark-pool (P4W10, #426):** equity/shorts/short_interest,
  equity/darkpool/otc.
- New keyless/free-key providers wired into routes: OECD, IMF discovery, Ken
  French Data Library, FINRA — plus deep FMP/FRED/Federal-Reserve/BLS/EconDB/
  SEC/CBOE/NASDAQ/Deribit expansion.

### Changed

- **Scoreboard truth-up (P4W11, #428):** `docs/roadmap/openbb-gap-matrix.md`
  (P4 roll-up + per-row Status) and `docs/products/openbb-parity.md` refreshed
  so every OpenBB command reads as done (with its route) or deferred-with-reason;
  the Part 2 implementation-layer rows are marked as a superseded historical
  planning record.

### Fixed

- Gemini-code-assist review findings folded forward each wave: SEC company_facts
  single-pass deserialization (avoids an intermediate `serde_json::Value` on
  10–20 MB filings, #421); OECD `limit` semantics now keep the most-recent N
  observations (#423); SEC `form_type` path-fallback + in-loop allocation
  hoisting (#426); FMP transcript error variant + executive empty-string
  filtering (#420); CBOE / Ken French allocation and readability fixes.

## [1.4.0] - 2026-06-13

The **intelligent-knowledge release.** Bundles every change merged to `main`
after the v1.3.0 tag: the knowledge-system-2 program (33 stories across the
Ease → Autonomy → Market → Finesse → Learning/Trust phases), the close-out of
OpenBB-parity Phase 3 (catalog at 131 routes / 98 provider candidates), plus
reliability and dependency hygiene. MINOR release — all additions are
backward-compatible; no protocol/persistence/operator-contract breaks. See
[`docs/release/v1.4.0-notes.md`](docs/release/v1.4.0-notes.md).

### Added

- **Knowledge system — ease of use (K-E):** zero-config first run with an
  in-memory default and actionable Bolt errors (#358); `tdw.kg.status`
  observability across MCP/REST/CLI (#360); public ingestion through a single
  `KnowledgeIndexer` seam (#359); offline `tdw kg demo` walkthrough (#378).
- **Knowledge system — autonomy (K-L):** rules + inference hosted in the daemon
  (#365); host-bound agent identity via session principals (#376); scheduled
  retrieval evals + drift alarm (#375); consolidation tick draining feedback and
  persisting plans (#377); gated auto-materialization sweep (#380); scheduled
  KnowledgeFeed pipelines (#382).
- **Knowledge system — market-grade (K-M):** LLM extraction-as-proposals,
  cost-bounded and gate-routed (#401); episodic memory of agent transcripts as
  searchable temporal episodes (#396); `tdw.kg.answer` cited GraphRAG synthesis
  (#404); contradiction-driven temporal invalidation (#392); published
  knowledge benchmark suite — recall/MRR/latency, nightly, drift-stamped (#405);
  knowledge graph-visualization Workspace widget (#415).
- **Knowledge system — finesses (K-X):** `tdw.kg.why` + `tdw.kg.diff`
  provenance chains & time-travel diffs (#362); trust-dial retrieval by
  provenance class (#394); self-narrating digest + staleness surfacing (#407);
  open questions that self-answer + negative knowledge (#403); knowledge
  watchlists with change-driven alerts (#398); first-class analyst findings
  capture + linking (#366); thesis tracking with temporal health (#391);
  session→findings distillation as proposals (#413); portable research-trail
  export — JSON + Markdown (#412).
- **Knowledge system — learning & trust (K-R):** lessons-as-proposals via B9 +
  walk-forward eval (#406); skill lifecycle + eval tournaments (#408);
  eval-gated parameter self-tuning (#409); deterministic motif mining + analogy
  recall (#381); pattern→rule induction through the gate (#400); per-fact
  confidence from corroboration + survived contradiction (#397); walk-forward
  knowledge validation replay harness (#393); governed, reversible forgetting to
  a cold plane (#411).
- **OpenBB parity Phase 3:** `tdw-analytics-portfolio` metrics (#361);
  `tdw-provider-imf` IMF SDMX-JSON macro series (#367); EconDB series (#369);
  `tdw-provider-famafrench` Ken French factors (#355); estimates breadth —
  price-target consensus + forward analyst estimates (#371); XLSX export from
  any result envelope via pure-Rust `rust_xlsxwriter` (#372).

### Changed

- Live `real-engines` feature chain wired with eager graph-backend validation
  at boot (#368).
- CFTC reads its app token via the shared `read_optional_key` helper (#363).
- OpenBB-parity P3 cutover: roll-up scoreboard + route-count refresh, and the
  niche-provider deferral decision documented in the gap matrix (#373, #374).
- Dependency bumps: candle-core/candle-transformers 0.10.2 (#385, #386),
  prost 0.14.4 (#389), zip 7.2.0 (#390), codecov-action 7 (#384),
  setup-qemu-action 4 (#383).
- CI review-gate checklist codified (release/schema-drift/clock/rebase gaps) in
  `docs/review-gate.md` (#417).

### Fixed

- Deterministic clock for `McpServer` — unbreaks date-brittle thesis tests
  (#414).
- Corrected Julian day math, scoped the contradiction edge scan, and narrowed
  the feed-indexer lock (Gemini-flagged escapes) (#399).
- Wired the real graph backend with eager validation, surfacing boot-time
  misconfiguration instead of failing lazily (#368).
- Clippy pedantic+nursery ratchet held at 0 across the release (#370, #379,
  #395, #402, #410, #416).

## [1.3.0] - 2026-06-11

The self-hosted warehouse release: Product ② (the ClickHouse/Qdrant/Meilisearch
warehouse stack) reaches a production-ready baseline — hardened compose
configuration, operator runbooks, OIDC IdP integration guide, cross-backend
behavioral conformance tests, a verified warehouse-install walkthrough, and a
nightly ingestion soak. See
[`docs/release/v1.3.0-notes.md`](docs/release/v1.3.0-notes.md).

### Added

- **Warehouse install + 15-minute eval path** (#288):
  `docs/products/warehouse-install.md` — compose `full` profile quickstart,
  checkpoint-gated evaluation sequence (live data flowing in ≤15 min), and
  honest caveats on what requires a keyed provider or additional setup.
- **Backup/restore + upgrade runbooks** (#289):
  `docs/release/backup-restore-runbook.md` and
  `docs/release/upgrade-runbook.md` — xtask-grounded procedures including the
  dry-run planner (`xtask migrate`), volume backup steps, and zero-downtime
  rolling-upgrade guidance.
- **OIDC IdP setup guide** (#290): `docs/release/oidc-idp-setup.md` — mapping
  tables for Keycloak, Auth0, and Microsoft Entra; fail-closed drill checklist;
  token-rotation runbook; claim values machine-verified against the auth
  contract.
- **Cross-backend conformance harness** (#283, #284, #285): parametrised
  behavioural test suites run over every backend (in-memory, SQLite, PostgreSQL)
  for the worker queue, outbox, and cost-ledger. Caught and fixed a real
  duplicate-enqueue divergence between the in-memory and durable backends (#283)
  — identical semantics are now an executable, CI-enforced guarantee.
- **Nightly Binance→ClickHouse ingestion soak** (#293): bounded soak job in
  `nightly.yml` that drives the live Binance websocket feed into ClickHouse and
  asserts row counts. Geo-block-tolerant: a reachability probe gates the soak
  steps and emits a structured skip summary rather than a hard failure when the
  exchange is unreachable from the runner.
- **Evaluation criteria for financial-data MCP servers** (#291):
  `docs/products/evaluation-criteria.md` — falsifiable 10-criteria checklist
  with every FinX cell repo-pinned; written as a durable evaluation framework,
  not a claims document.
- **Commercial support + SUPPORT.md** (#300): `SUPPORT.md` with community and
  three commercial tiers; announcement pricing paragraph live. Closes D2.

### Fixed

- **Nightly geo-block tolerance + e2e compose token** (#294): live-smoke now
  skips on Binance HTTP 451 (geo-block) rather than failing; the e2e-full job's
  required `TDW_MCP_HTTP_TOKEN` is now set in the job environment.

### Changed

- **Compose `full` profile hardened** (#282): all images pinned with rationale
  comments; healthcheck coverage expanded from 6 to 9 services, cross-matched
  to CI wait-steps; named volumes replacing anonymous mounts; `.env.example`
  fully covers every variable consumed by the stack.

## [1.2.0] - 2026-06-10

The "live data, for real" release: the MCP financial-data server now serves
live market data end to end, verified by driving it as a real MCP client. See
[`docs/release/v1.2.0-notes.md`](docs/release/v1.2.0-notes.md).

### Added

- **`live` feature on `tdw-mcp`** (#269). Swaps the offline Yahoo fixture for
  the real HTTP fetcher and registers every live HTTP provider (34 providers /
  51 fetcher endpoints vs 3 offline). GHCR images (#271) and tagged release
  binaries (#272) build with it, so distribution artifacts serve real data.
- **Generic provider dispatch: `tdw.provider.fetch`** (#274). Any compiled-in
  fetcher is callable by `(provider, endpoint)`; a drift-guard test pins
  dispatch completeness against the registry. Previously only yahoo + fileset
  were reachable from `tools/call`.
- **Yahoo cookie+crumb handshake** (#268). v10 `quoteSummary` / v7
  quote/options endpoints reject anonymous requests with 401 "Invalid Crumb";
  the fetcher now performs the browser handshake lazily on 401/403 only, so
  offline tests never touch the network.
- **Live-test coverage** (#267, #268): CoinGecko's documented live test now
  exists; Binance gained a live websocket subscribe test (first real BTCUSDT
  trade tick asserted).
- **MCP quickstart** (#271, #272): `docs/products/mcp-quickstart.md` — GHCR
  one-liner, from-source build, Claude Code/Desktop wiring, per-provider
  API-key table. Plus the tool-surface audit that drove this release
  (`docs/products/mcp-tool-surface-audit.md`).
- **Nightly live-smoke job** (#275): provider live tests + MCP E2E live bars.
- **Per-crate `pedantic`+`nursery` deny** in the 58 lint-clean crates (#257)
  and a **performance benchmark + regression ratchet** in `xtask` (#263).
- **Provider data expansions** (L2.x): FRED macro/rates/spreads/fixed-income
  cluster (#251), Yahoo profile/quote/discovery/options/futures (#254), FMP
  fundamentals normalized to the L1.4 models (#252).
- **Application function jobs** behind the off-by-default `functions` feature:
  `RoutingJobHandler` (#246), worker-side execution (#248), and enqueue on
  `user.created` via `FunctionEnqueuer` (#249).

### Fixed

- **SEC EDGAR live conformance** (#267): CIKs normalized from the zero-padded
  wire form; XBRL revenue extraction falls back through the post-ASC-606
  us-gaap concepts (`RevenueFromContractWithCustomerExcludingAssessedTax`,
  `Revenues`, `SalesRevenueNet`).
- **`block_on` reactor panic** (#269): the noop-waker busy-poll panicked with
  "there is no reactor running" the moment a live reqwest fetcher ran, and a
  naive runtime rebuild panicked inside `#[tokio::main]` callers; the helper
  is now runtime-context-aware.
- **Hermetic dispatcher tests + CI disk space** (#277): three dispatcher
  tests silently depended on live Yahoo reachability under
  `all-http-providers` (verified by running them with the network strangled);
  and docker-heavy CI jobs now free ~30GB of runner disk before building the
  release+`live` images, fixing the ENOSPC that masqueraded as a cargo-chef
  panic.
- **Container Image CI timeouts** (#270): buildx GHA layer cache (scoped per
  binary, shared by the scan and push builds) + a 120-minute cold-cache
  backstop; main-push image builds previously died at the 60-minute job
  timeout building the workspace under QEMU.

### Changed

- **MCP catalog honesty** (#273, #274): `.sample` evidence tools are hidden
  from the default `tools/list`, and the server discloses fixture-vs-live
  data mode in `initialize` instructions and tool descriptions.
- **G005–G007 hardening reconciled to main** (#264, #265): bounded HTTP
  clients via the shared `build_client` (10s connect / 30s request timeouts),
  validator coverage for the new protocol ops, and the hooks deny→ask
  reconciliation.
- **Dependency hygiene** (#266): unused deps pruned (eval-runner `serde_json`;
  service `tokio-util`, `toml`) and a `needless_return` cleanup in `tdw-core`.

## [1.1.0] - 2026-06-08

Security, observability, feature-platform, and production-readiness release on
top of `v1.0.0`. Hardens authentication (cryptographic OIDC, constant-time token
comparison, loopback-default daemon bind), wires real storage/compute engines
into the `live` profile by default, completes registry-driven dispatch, lands
the worker dead-letter operator surface and full ops/health surface, and adds
the first OpenBB-gap-closure layer (standardized result envelope, cluster data
models, shared query params, logical-endpoint resolution, symbology). It also
builds out the application feature platform — alert engine, transactional and
broadcast email, news aggregation, a multi-step function/cron spine, first-party
identity/session/password stores, the Finnhub provider, an LLM fallback/router
with error classification, and tool-execution autonomy gating with a hash-chained
receipt log — on top of a workspace-wide documentation, self-improve, and
dependency-hygiene sweep.

### Added

- **Real engines by default in the `live` profile** (#157). `tdw-service-api`'s
  `AppState` now wires the real ClickHouse / Postgres / Qdrant / Meilisearch /
  S3 engines (behind feature gates) instead of in-memory stand-ins when the
  `live` Compose profile is selected, so the deployed stack exercises the
  production storage/compute paths. The default offline build is unchanged.
- **Registry-driven dispatch end to end** (#158). Ingest is now driven through
  the provider registry, a `ToolRegistry` routes tool/MCP calls, and the wasm
  UDF runtime is reachable from the daemon dispatch path — closing the gap
  between the registered provider/tool set and what the daemon can actually
  execute.
- **Worker dead-letter operator surface** (#153). `tdw-worker` gains
  `dead-letter list` / `dead-letter replay` CLI subcommands for inspecting and
  re-enqueueing dead-lettered jobs, plus a bounded-concurrency clamp so
  `TDW_WORKER_CONCURRENCY` cannot exceed a safe ceiling. Documented in
  `docs/release/worker-deployment.md`.
- **Cryptographic OIDC verification** in `tdw-auth-oidc` (#150). New
  `verify_jwt` / `verify_jwt_strict` verify a compact JWT's signature against
  supplied verifying keys (RS256/ES256, resolved by `kid`) and enforce
  `exp`/`nbf`/`iat` (60s clock-skew leeway), issuer, and audience — failing
  closed on any error. The `none` pseudo-algorithm and HMAC tokens are rejected
  (alg-confusion / `alg:none` defence). Built on `jsonwebtoken` (default `ring`
  backend, already a vetted transitive dependency). The existing structural
  claim/JWKS checks remain as a pre-filter. Remote JWKS fetch stays out of
  scope: verifying keys are supplied from the configured JWKS.
- **Ops/health surface + graceful drain** (#161, G002). `/health`, `/ready`,
  and `/metrics` endpoints plus coordinated graceful drain for the daemon,
  worker, and MCP server, so the deployed stack is probe- and shutdown-aware.
- **Price-alert engine** (#180, #187, #199). A `PriceAlert` domain model with a
  Postgres migration and alert stores (#180), a `tdw-alert-evaluator` price-alert
  evaluation function on a 5-minute cron (#187), and owner-scoped alert CRUD
  daemon ops in `tdw-service-api` (#199).
- **Function/cron spine** (#177, #185, #186). A `tdw-cron` recurring-trigger
  spine over the worker queue (#177), a multi-step `tdw-functions` registry with
  per-step memoization (#185), and cron/event triggers wired to worker-job
  execution (#186).
- **Transactional + broadcast email** (#183, #201). `tdw-email` transactional
  SMTP send with HTML template fill (#183), plus a marketing/broadcast client
  behind a `broadcast` feature (#201).
- **News aggregation policy layer** (#204). A new `tdw-news-compose` crate that
  aggregates and composes news under an explicit policy layer.
- **First-party identity stores** (#193, #205). A first-party user + password
  store using argon2 (#193) and a session store (#205) in `tdw-identity`.
- **Finnhub provider** (`tdw-provider-finnhub`, #192). Company profile and quote
  fetchers following the canonical provider pattern.
- **LLM fallback, router, and error classification** (#169, #194, #195).
  `tdw-llm` gains a `FallbackModel` primary→secondary provider wrapper (#169), a
  credential-aware provider router (#195), and retryable-vs-permanent error
  classification (#194).
- **Tool-execution autonomy gating + receipt log** (#196, #197, #198).
  `tdw-tool-exec` gates execution on `ToolEffect` risk via an opt-in
  `AutonomyLevel` (#196), keeps an opt-in hash-chained tool-receipt log (#197),
  and validates call arguments against an opt-in arg schema before dispatch
  (#198).
- **FunctionRegistry over HTTP** (#188). `tdw-app-server` exposes the
  `FunctionRegistry` over HTTP with HMAC request signing.
- **Live `QuoteSnapshot` read path** (#179). A `QuoteSnapshot` domain type plus
  an uncached live read path for quote data.
- **OpenBB-gap-closure layer 1** (#176, #173, #190, #191). The first layer of
  the OpenBB clean-room gap-closure plan (analysis + layered plan in #176): a
  standardized result envelope and cluster data models, shared query-param
  normalization with a yahoo/fred pilot (L1.3, #191), logical-endpoint provider
  resolution (L1.5, #190), and a pure ticker-symbology normalization crate
  `tdw-symbology` (#173).

### Changed

- **Constant-time bearer-token comparison** on the MCP Streamable HTTP layer
  (#150). `TDW_MCP_HTTP_TOKEN` validation now compares tokens via `subtle`'s
  `ConstantTimeEq` over fixed-width digests instead of `==`, removing the
  timing side channel (and not leaking token length).
- **Safe daemon TCP defaults** (#150). The daemon TCP transport already
  defaults to loopback (`127.0.0.1:7878`) when `TDW_DAEMON_TCP_BIND` is unset;
  it now logs a prominent `SECURITY WARNING` at startup when bound to a
  non-loopback address with no auth-backed policy attached. **Operator note:**
  deployments that previously relied on an implicit non-loopback bind must set
  `TDW_DAEMON_TCP_BIND` explicitly and attach an auth-backed policy.
- **Partial OIDC config is now a hard startup error** (#150). A
  partially-configured `prod`/`production` boot (some but not all `TDW_OIDC_*`
  set, or invalid JWKS/claims) makes the daemon **refuse to start**, with a
  diagnostic listing every missing variable. A fully-unset OIDC config keeps the
  existing fail-closed (starts, dispatches return `Failed`) behavior.
  `OidcPolicyError` gained `MissingEnvVars(Vec<&'static str>)` (replacing the
  single-var `MissingEnvVar`).
- **CI: live-stack + tools smoke jobs, aarch64 release leg, multiarch images**
  (#155). A new `live-stack` workflow brings up the Compose stack and runs the
  smoke path; the CI tools job covers the `tdw-cli`/`tdw-mcp` surface; the
  release workflow gains an `aarch64-unknown-linux-gnu` build leg; and container
  images are now built multiarch.
- **CI: concurrency groups** (#200) cancel superseded PR runs so only the latest
  push per branch consumes runners.
- **`jsonwebtoken` 9.3.1 → 10.4.0** on the `rust_crypto` backend (#163), keeping
  the OIDC verifier on a current, maintained JWT implementation.

### Security

- The OIDC, constant-time token comparison, and loopback-default daemon-bind
  changes (#150) collectively harden the production authentication and transport
  posture. See *Upgrade notes* in
  [`docs/release/v1.1.0-notes.md`](docs/release/v1.1.0-notes.md) for the
  breaking-for-exposed-deployments details.
- `cargo-deny` now ignores `RUSTSEC-2026-0173` (proc-macro-error2 unmaintained),
  a build-time-only transitive advisory with no runtime exposure (#202).

### Performance

- **Verification wall time halved** (#178). Test-target gating, a doctest-harness
  purge, and fixture shrinking cut workspace verification wall time by ~54% over
  three self-improve iterations, without reducing coverage.

### Docs

- **Consolidated `TDW_*` environment reference + operator setup** (#149). New
  [`docs/CONFIGURATION.md`](docs/CONFIGURATION.md) is the single source of truth
  for every `TDW_*` variable, with a rewritten `.env.example`, a
  `secrets-and-tls.md` runbook, and `compose-setup` helper scripts.
- **Comprehensive per-crate README/ARCHITECTURE/examples across all crates**
  (#164–#168, #170–#172). A workspace-wide documentation sweep adding a README,
  an ARCHITECTURE note, and runnable examples to every crate — provider crates
  (batches A/B/C, including the ws and proto crates), domain/data, storage and
  persistence, AI (llm/embed/agent/udf), service/binary, and core infra.
- **OpenBB clean-room gap analysis + layered closure plan** (#176). A roadmap
  document that scopes the OpenBB feature gap and the layered plan to close it.
- **Lint-debt sweep.** `missing_const_for_fn` resolved across 16 crates
  (#156, #160) and `too_long_first_doc_paragraph` across 4 crates (#162).
- **Release roster + 1.0 gap-audit closure.** Crate-readiness roster sync (#182)
  and the 1.0 gap-audit closure (#159).

### Internal

- **Self-improve campaigns.** Provider HTTP fetchers deduplicated via a shared
  core (−60% duplication, #175); workspace verification time reduced (−54%, #178,
  see *Performance*); line coverage raised across `tdw-backend`/`tdw-service-api`/
  `tdw-core` (#181) and daemon-serving paths covered (#203); and 17
  code-reference-free workspace dependency edges removed (351 → 334, #184).

## [1.0.0] - 2026-06-07

### Added

- Release 1.0 readiness hardening: Yahoo's real HTTP fetcher is now selectable
  through `tdw-service-api`'s provider feature set and included in
  `all-http-providers`.
- Deterministic coverage for the `tdw-bootstrap`, `tdw-cli`, and `tdw-proto`
  crates so the batch backlog no longer treats them as untested leaf binaries.

### Changed

- Updated release-facing README status text to reflect the existing tag history
  and the active `v1.0.0` readiness branch.

## [0.10.0] - 2026-06-06

Protobuf market-data types and full data-provider wiring: every standalone
`tdw-provider-*` crate is now registrable in the service dispatcher behind
per-provider feature gates, with the default build still fully offline.

### Added

- **`tdw-proto` crate** (#139). Protobuf bindings for the core market-data
  types — `OhlcvBar`, `Tick`, `PriceLevel`, `OrderBookSnapshot`, and the
  `MarketDataEnvelope` `oneof` wrapper — generated from
  `proto/market_data.proto`. The generated bindings are vendored
  (`src/finance.gen.rs`) so the crate builds with no system `protoc` and no
  build-time codegen; the runtime depends only on upstream `prost`.
- **All 30 standalone data providers wired into the dispatcher** (#140, #141).
  `tdw-service-api::default_registry()` can now register every
  `tdw-provider-*` crate behind a per-provider cargo feature
  (`provider-<name>`) plus an `all-http-providers` aggregate. With no features
  the default registry stays offline and registers exactly 3 providers
  (fileset, yahoo, mock-ws); `all-http-providers` registers 50 distinct
  `(provider, endpoint)` entries. Seven providers (adanos, benzinga, cboe,
  deribit, eia, glassnode, seeking-alpha) were converted to the canonical
  `tdw_core::Fetcher` trait. The CI lint job now clippy- and test-checks the
  `all-http-providers` feature so the wired registry cannot silently regress.
- **`TDW_DAEMON_OPEN_POLICY` escape hatch and a worker concurrency default of
  4** (#133).

### Changed

- Dependency bumps: `ratatui` 0.30.0 → 0.30.1 (#137), `aws-config`
  1.8.17 → 1.8.18 (#135), `aws-sdk-s3` 1.134.0 → 1.135.0 (#138), `chrono`
  0.4.44 → 0.4.45 (#136), and the `docker/build-push-action` GitHub Action
  6 → 7 (#134).

### Fixed

- `tdw-provider-tiingo` did not compile under `--features http`
  (`TiingoNewsArticle` lacked the `Serialize`/`Deserialize`/`JsonSchema`
  derives required by `DataModel`); fixed and now covered by the new
  `all-http-providers` CI checks (#141).

## [0.9.0] - 2026-05-31

Ten additional data providers completing the gap-analysis coverage sweep
(waves 4 and 5). All follow the canonical `tdw-provider-polygon` pattern:
offline `lib.rs` with validation and mock fetcher, feature-gated
`http_fetcher.rs` with real HTTP + serde deserialization, and cassette +
live integration tests gated by `TDW_*_LIVE=1`.

### Added

- **OECD provider** (`tdw-provider-oecd`, #130). SDMX-JSON endpoint for
  international economic statistics; no API key required.
- **Velodata provider** (`tdw-provider-velodata`, #130). Crypto derivatives
  analytics — funding rates, liquidations, and open interest across
  Binance/Bybit/OKEx/Hyperliquid (`TDW_VELODATA_API_KEY`).
- **ECB provider** (`tdw-provider-ecb`, #130). ECB Statistical Data Warehouse
  — EUR exchange rates and €STR interest rates; no API key required.
- **TMX provider** (`tdw-provider-tmx`, #130). Toronto Stock Exchange equity
  quotes and MX options chain; no API key required.
- **GeckoTerminal provider** (`tdw-provider-geckoterminal`, #130). DeFi/DEX
  on-chain pool data — OHLCV, liquidity, token metrics; no API key required.
- **CCData provider** (`tdw-provider-ccdata`, #131). CryptoCompare daily OHLCV
  and asset metadata (`TDW_CCDATA_API_KEY`).
- **Adanos provider** (`tdw-provider-adanos`, #131). Social sentiment aggregator
  covering Reddit, X, news, and Polymarket events (`TDW_ADANOS_API_KEY`).
- **FINRA provider** (`tdw-provider-finra`, #131). FINRA short interest and
  weekly OTC market summary; public API, no auth required.
- **Seeking Alpha provider** (`tdw-provider-seeking-alpha`, #131). Analyst
  articles and quant/author ratings via RapidAPI
  (`TDW_SEEKING_ALPHA_API_KEY`).
- **Deribit provider** (`tdw-provider-deribit`, #131). Crypto options and
  futures — instrument listing, order book with Greeks, and perpetual funding
  rate history; public endpoints, no auth required.

## [0.8.0] - 2026-05-31

Production auth, embeddable backend, agent learning, and the first fifteen
data providers (waves 1–3). Thirteen user-visible runtime/provider changes
since v0.7.0, so this is a `MINOR` release.

### Added

- **Production OIDC policy** (#116, #119). `TDW_OIDC_*` env vars wire an
  auth-backed policy when `TDW_PROFILE=prod`; observable via a `/healthz`-style
  endpoint. Validation is structural (claim/JWKS consistency), not
  cryptographic.
- **Postgres + Clickhouse MCP servers** (#120). Project-scoped `.mcp.json`
  wires `postgres-mcp` (Pro, read-only) and `mcp-clickhouse` via
  `uvx --python 3.13` so Claude can query live local backends directly.
- **Unified embeddable backend** (`tdw-backend`, #121). Library + binary facade
  over the full warehouse stack; dual sync/async API for embedding or running
  standalone.
- **Durable agent learning** (`tdw-agent-learning`, #122). Knowledge index,
  memory-consolidation loop, and eval feedback cycle with adaptivity gate.
- **Data providers — wave 1** (#123–#127). Databento (CME Globex futures tick
  data), FMP (fundamentals + OHLCV), SEC EDGAR (filings, XBRL), Tiingo (OHLCV
  + news), CoinGecko (crypto market cap / dominance).
- **Data providers — wave 2** (#128). Alpha Vantage, CBOE (options/VIX),
  Benzinga (news + earnings calendar), NASDAQ Data Link, AkShare (Chinese
  A-share + HK markets).
- **Data providers — wave 3** (#129). Tradier (equities + options chains), EIA
  (US energy spot prices), Glassnode (on-chain MVRV/LTH/NUPL), Trading
  Economics (global macro calendar), BLS (US CPI + employment).

## [0.7.0] - 2026-05-30

Daemon hardening and durability follow-ups. The commits in `v0.6.0..HEAD`
include four user-visible runtime/storage changes, so per the pre-1.0 policy
this is a `MINOR` release.

### Added

- **Per-request WASM limits** (#110). `UdfRequest` gains an optional
  `wasm_limits` (`WasmLimitsRequest { fuel, max_memory_bytes, max_memories }`)
  so a caller can give an untrusted UDF a smaller fuel/memory budget per call.
  Values can only **tighten** a limit — they are clamped to the runtime default
  ceiling, never raised above it — so this is a budget knob, not a DoS lever.
  The field is serde-default + skip, so existing `udf.run` payloads
  deserialize/serialize unchanged.
- **Postgres-backed daemon session + rollout stores** (#112). New
  `daemon-postgres` feature plus `SessionBackend` / `RolloutBackend` enums on
  `AppState` and a new `tdw_rollout::PgRollout`. With the feature built **and**
  `TDW_DAEMON_PG_URL` (or `DATABASE_URL`) set, the daemon's session/cost ledger
  and rollout archive persist to Postgres instead of SQLite + a JSONL file, so
  they survive container restarts. Wired into the `live` compose daemon (image
  built `--features daemon-postgres`); default builds are unchanged.
- **Worker concurrency** (#111). `ServeConfig.max_concurrent` +
  `TDW_WORKER_CONCURRENCY` let `tdw-worker --serve` drive up to N in-flight jobs
  at once via `FuturesUnordered` (no extra threads). Default `1` preserves
  strictly serial behavior; shutdown stops new leases and drains in-flight work.
  The `live` worker runs at concurrency 4.

### Changed

- **Daemon honors `TDW_PROFILE`** (#109). `tdw-service` `load_config` now
  applies the `TDW_PROFILE` env var (e.g. the `live` stack's `docker`), so the
  profile-driven local policy attaches as intended and live dispatches resolve
  instead of failing closed. The startup log reports the actual attached-policy
  state rather than a fixed "no policy" message.

### Internal

- Multi-session git guardrails: pre-push hook + house-rules doc (#107); removal
  of files accidentally committed in #106 (#108); clippy pedantic/nursery
  warning cleanup 301 → 14 (#106). CI lint now also compile-checks the
  `daemon-postgres` Postgres store paths.

## [0.6.0] - 2026-05-29

Live streaming ingest plus the full long-running deployment surface. The 5
commits in `v0.5.0..HEAD` are user-visible runtime/protocol/deployment work, so
per the pre-1.0 policy this is a `MINOR` release.

### Added

- **Postgres-backed worker `--serve`** (#101). `tdw-worker --serve` selects its
  durable backend from the environment: `PgWorkerQueue` when built
  `--features postgres` with `TDW_WORKER_PG_URL`/`DATABASE_URL`, otherwise the
  SQLite default. `run_serve` is now generic over `ServeQueue`.
- **Long-running services in the `live` compose profile** (#102). A long-running
  `tdw-service` daemon (`TDW_DAEMON_TCP_BIND` lets it bind `0.0.0.0:7878` for
  cross-container reach), a `tdw-mcp --streamable-http` server (daemon-routed),
  and a Postgres-backed `tdw-worker --serve` (worker image `FEATURES` build-arg).
- **End-to-end streaming ingest** (#100). `run_ws_ingest` + `spawn_stream_ingest`
  make a `Streamer` reachable as a cancellable background ingest task draining
  into the OLAP engine; restart-safe via the content-addressed dedup token
  (at-least-once, no materialized-view double-count).
- **Live Binance trade feed + indicators** (#104). `tdw-provider-binance`
  `BinanceTradeStreamer` (live `wss://stream.binance.com` behind a `ws` feature;
  deterministic offline `decode_trade_frame` seam), `Op::StreamStart`/`StreamStop`
  protocol ops with `tdw-acp` validation and dispatcher routing, plus fixed-N
  volatility and an exact Wilder RSI UDF.

### Fixed / Security

- **Hardened `live` MCP/daemon exposure** (#103). `TDW_MCP_HTTP_TOKEN` is now
  required (no weak default) so a host-published, non-loopback MCP bind is always
  authenticated; the `tdw-service` daemon is internal-only (host port publication
  dropped) since its transport is unauthenticated plaintext.

### Notes

- The `live` profile end-to-end run requires a Docker daemon. `tdw-service`
  boots with no policy attached, so dispatched operations return `Failed` until a
  policy is wired.
- `Cargo.toml` keeps `publish = false`; the workspace `version` field is not
  bumped per release (releases are tag-driven).

## [0.5.0] - 2026-05-29

Adds a streaming market-data warehouse on ClickHouse. The 9 commits in
`v0.4.0..HEAD` include a major new ingest/analytics feature, so per the pre-1.0
policy this is a `MINOR` release.

### Added

- **Streaming market-data warehouse on ClickHouse** (#87). Real streaming
  ingest that persists (`dispatch_ingest` now writes the fetched batch with an
  idempotent `async_insert` + dedup-token helper), a `tdw-provider-ws`
  tokio-tungstenite streamer, and raw tables (`raw.tick`/`trade`/`quote`/
  `book_level` with `DateTime64(9)`, DoubleDelta+ZSTD codecs, `LowCardinality`
  dims, monthly partitions, TTL, dedup windows). A tier of always-fresh
  "FlowField" incremental materialized views (`AggregatingMergeTree` + reader
  views): OHLC 1m/5m/1h/1d (per-venue + consolidated), VWAP, daily return, top
  movers, trailing 52w high/low and 30d volatility, quote mid/spread, book
  best-bid/ask + depth + imbalance, daily news sentiment, and technical
  indicators. A reference-entity model (Postgres master + ClickHouse
  dictionaries for `dictGet` enrichment: symbol info, trading calendar,
  corporate actions, FX rates). Optional `tdw-storage-broker` (feature-gated
  pure-Rust `rskafka` write sink + Kafka-engine consumer migrations). Validated
  against live ClickHouse 26.6 + PostgreSQL 18.

### Changed

- **CI** now lints and tests the opt-in UDF runtime features
  (`tdw-udf-wasm --features wasmi`, `tdw-sandbox`/`tdw-service-api --features
  udf-wasm`), closing a blind spot where those paths were never built in the
  default matrix; `dependabot.yml` ignores wasmi semver-major bumps (#97).
- **Dependency bumps**: `aws-sdk-s3` 1.133→1.134 (#96), `uuid` 1.23.1→1.23.2
  (#98), and GitHub Actions `setup-buildx`/`upload-artifact`/`download-artifact`/
  `login-action`/`attest-build-provenance` (#90–#94).

### Notes

- `Cargo.toml` keeps `publish = false`; the workspace `version` field is not
  bumped per release (releases are tag-driven).

## [0.4.0] - 2026-05-29

Runtime follow-ups on top of `v0.3.0`: the worker now executes real work, and
the hardened UDF runtime is wired end to end. The 3 commits in `v0.3.0..HEAD`
are user-visible runtime changes, so per the pre-1.0 policy this is a `MINOR`
release.

### Added

- **Worker daemon dispatch.** `tdw-worker --serve` gains a `DaemonJobHandler`
  that submits each leased job's `OpEnvelope` to the configured TDW daemon via
  `tdw-app-client` and maps the terminal event onto the job contract
  (`Completed` -> complete; `Failed`/`Cancelled`/transport error -> retry then
  dead-letter). Selected by `TDW_WORKER_DISPATCH=daemon` / `TDW_WORKER_DAEMON_*`
  (TCP/UDS/HTTP-SSE); the offline `LoggingAckHandler` stays the default. (#85)
- **Wasm UDF string ABI.** `tdw-udf-wasm` adds `execute_wasm_string` (feature
  `wasmi`): a linear-memory string-in/string-out ABI (guest exports `memory` +
  `alloc(i32)->i32` + `<func>(in_ptr,in_len)->i64` returning packed
  `(out_ptr,out_len)`) under the existing fuel/memory/deny-imports hardening.
  All guest memory access uses wasmi's checked `Memory::read`/`write`, so a bad
  pointer/length or non-UTF-8 output yields `BadAbi`, never a host panic. (#86)
- **Sandbox routing to the hardened runtime.** `tdw-sandbox`'s `udf-wasm`
  feature now enables `tdw-udf-wasm/wasmi`; a `UdfRuntime::Wasm` request whose
  `source` base64-decodes to a real wasm module runs through
  `execute_wasm_string`, otherwise it falls back to the deterministic fixture.
  This completes the UDF runtime hardening scope (step #5). (#88)

### Notes

- Default `cargo test --workspace` stays offline: the worker daemon path and the
  `wasmi` UDF path are both opt-in (env / feature); without them the worker uses
  the ack handler and the sandbox uses the built-in dispatcher.
- `Cargo.toml` keeps `publish = false`; the workspace `version` field is not
  bumped per release (releases are tag-driven).

## [0.3.0] - 2026-05-29

Runtime, sandbox, and live-backend hardening on top of `v0.2.0`. The 12 commits
in `v0.2.0..HEAD` add a supervised worker process, a real sandboxed UDF engine,
and a fully-bootstrapped live data backend - user-visible runtime and storage
work, so per the pre-1.0 policy this is a `MINOR` release.

### Added

- **Supervised worker process.** `tdw-worker --serve` / `--serve-once` run a
  `WorkerRunner` lease loop over the durable queue: lease (with payload) →
  run a `JobHandler` → complete, or fail with retry/dead-letter at
  `max_attempts`. In-flight jobs always finish on shutdown (the stop signal is
  observed only between jobs). Tunables via `TDW_WORKER_DB` / `TDW_WORKER_ID` /
  `TDW_WORKER_LEASE_TTL_MS` / `TDW_WORKER_POLL_MS`. (#72)
- **Real `wasmi` UDF runtime.** `tdw-udf-wasm` gains a `wasmi`-backed engine
  behind the opt-in `wasmi` feature (`execute_wasm_i64`): fuel metering
  (`FuelExhausted`), `WasmLimits` memory caps (`MemoryLimitExceeded`), and
  deny-by-default host imports (empty `Linker`). The deterministic fixture path
  stays the default. (#79)
- **Live backend expansion.** The `live` compose profile now brings up
  ClickHouse, Qdrant, and Meilisearch; `tdw-bootstrap` creates baseline schemas
  in each (ClickHouse `tdw` DB + marker table, Qdrant `tdw-default` collection,
  Meilisearch `tdw-default` index) alongside the Postgres/S3 bootstrap; and a
  long-running `tdw-worker --serve` service starts after bootstrap succeeds.
  `QdrantHttpEngine::ensure_collection` is now public and
  `MeilisearchHttpEngine::ensure_index` was added. (#81)
- **Test-policy hardening.** Mutation tooling (#71), the first loom concurrency
  model on `tdw-app-server` (#73), stable corpus-replay fuzz harnesses for six
  parser surfaces (#74), nightly `cargo-fuzz` targets with a CI smoke job
  (#75, #78), and an `xtask` pre-release fuzz+loom check recipe (#76) - closing
  TEST-POLICY-001 through 005.

### Changed

- Reduced clippy pedantic/nursery warnings across the workspace (#80, #83).

### Docs

- Added the full deployed-stack runbook (#77) and updated the data-backend
  runbook + transport-status matrix for the expanded `live` profile.

### Notes

- `Cargo.toml` keeps `publish = false`; the workspace `version` field is not
  bumped per release (releases are tag-driven).

## [0.2.0] - 2026-05-29

First substantial release after the early `v0.1.0`/`v0.1.1` packaging tags
(both cut on 2026-05-28). The 15 commits in `v0.1.1..HEAD` land the daemon
runtime, the MCP server surface, durable worker schedulers, and the live data
backend - user-visible runtime and protocol work, so per the pre-1.0 policy
this is a `MINOR` release.

### Added

- **Daemon runtime (ADR-0012).** Completed the P0-P8 daemon integration cycle
  and the stateful `tdw-mcp` stdio JSON-RPC MCP server (`initialize`,
  `tools/*`, `resources/*`, `prompts/*`, progress notifications, cancellation,
  and error paths).
- **MCP Streamable HTTP transport.** `tdw-mcp --streamable-http [bind]` serves
  the same MCP protocol over a local-first HTTP endpoint at `/mcp`, with Origin
  validation, `MCP-Protocol-Version` checks, header/body size bounds, and
  optional bearer auth via `TDW_MCP_HTTP_TOKEN` (#63).
- **Daemon-backed MCP tools.** `tdw.daemon.triage` and
  `tdw.daemon.query.submit` build `OpEnvelope` operations and route them
  through `tdw-app-client` to a live daemon, returning event evidence as
  structured MCP output; deterministic offline tools continue to run without a
  daemon.
- **MCP daemon-client transport expansion.** `tdw-app-client` submits daemon
  operations over TCP, `cfg(unix)` Unix domain sockets, and plain HTTP/SSE
  (`POST /op` + `GET /events`). Selection is configurable through
  `TDW_MCP_DAEMON_TRANSPORT`, `TDW_MCP_DAEMON_ADDR` (or `TDW_DAEMON_TCP_BIND`),
  and `TDW_MCP_DAEMON_TIMEOUT_MS`; unsupported endpoints (Windows UDS, HTTPS
  HTTP/SSE) fail closed.
- **Durable worker schedulers.** `tdw-worker` gains an embedded SQLite durable
  scheduler (priority leasing, lease expiry, retry/dead-letter, idempotent
  enqueue/complete, stats, `--durable-smoke`) and a distributed Postgres
  `PgWorkerQueue` behind the `postgres` feature that mirrors the same contract.
  The in-memory contract backend remains for offline tests.
- **G014 live data backend.** A `live` compose profile plus the `tdw-bootstrap`
  one-shot binary bring up Postgres + MinIO, apply the G013 Postgres schemas,
  and write/read back an S3 marker; documented in
  `docs/release/data-backend-runbook.md` (#47).
- **Protocol and integration test coverage.** Always-on `tdw-app-client`
  daemon-framing tests (length-delimited writes, empty/oversized frame
  rejection, terminal-event matching, HTTP/SSE submit-path derivation), an
  env-gated daemon-backed MCP integration test
  (`TDW_MCP_DAEMON_INTEGRATION_ADDR`), an env-gated durable Postgres worker test
  (`TDW_POSTGRES_TEST_URL`), and a CI worker Postgres-queue step.
- **Test-policy decisions encoded.** ADR `docs/adr/0014-test-policy-backlog.md`
  plus policy docs fix the mutation cadence (O24), the first loom model scope
  (O25), and the initial fuzz-target list (O26), with deferred enforcement
  tracked as `TEST-POLICY-001..005`.
- **Deployment guidance for the remaining product gaps.**
  [`docs/release/mcp-remote-deployment.md`](docs/release/mcp-remote-deployment.md)
  (remote MCP HTTP behind a TLS/OAuth reverse proxy) and
  [`docs/release/worker-deployment.md`](docs/release/worker-deployment.md)
  (`PgWorkerQueue` rollout, supervision, and lease/dead-letter monitoring).

### Changed

- **Bounded daemon connect.** `tdw-app-client` uses
  `TcpStream::connect_timeout` for validated TCP daemon endpoints so the
  configured timeout now covers connection establishment as well as read/write.
- **License.** Repository relicensed to dual `MIT OR Apache-2.0` (#68).
- **Reduced pedantic/nursery lint noise** across the workspace; tooling pins
  `--target-dir` on the clean-room audit and documents the WDAC gotcha (#66).

### Governance and tooling

- Agent rules tracked and worktree cleanup guarded (#48); patch-equivalent
  worktree branch deletion fixed (#49); `.mcp.json` gitignored (#61); the
  dormant `create-private-repo` bootstrap script removed (#64); the production
  functional gate documented (#46); and validated ULTRAQA characterization
  coverage salvaged.

### Notes

- `Cargo.toml` keeps `publish = false`; the workspace `version` field is not
  bumped per release (releases are tag-driven, as they were at `v0.1.1`).

## [0.1.1] - 2026-05

Packaging and fix release on top of the `v0.1.0` G014 release surface. See the
`v0.1.1` tag and its GitHub release for the packaged archives, checksums, and
attestations.

## [0.1.0] - 2026-05

Initial tagged release. G014 release-packaging surface for `tdw-service`,
`tdw-cli`, `tdw-mcp`, and `tdw-worker`: multi-target binary archives with
checksums and build-provenance attestations, plus scanned GHCR container
images. See `docs/release.md` for the full artifact and image policy.

[Unreleased]: https://github.com/xrey167/FinX-Plattform/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/xrey167/FinX-Plattform/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.10.0...v1.0.0
[0.10.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/xrey167/FinX-Plattform/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/xrey167/FinX-Plattform/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/xrey167/FinX-Plattform/releases/tag/v0.1.0
