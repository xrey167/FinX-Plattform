# Evaluating financial-data MCP servers: criteria and FinX's evidence

When you wire market data into an AI agent, the server becomes part of your
trading/research loop's trust chain. This page proposes a concrete evaluation
checklist for *any* financial-data MCP server — then shows, line by line,
where FinX-Plattform's claims come from. Run the same checklist against any
alternative (OpenBB-based bridges, yfinance wrappers, exchange-specific
servers); every FinX cell links to something you can re-execute or read.

## The checklist

| # | Criterion | Why it matters for agents | FinX evidence |
|---|---|---|---|
| 1 | **Live-verified continuously** — does CI exercise the real provider APIs, or only fixtures? | Provider APIs drift (auth handshakes, concept names, padding). Fixture-only servers rot silently. | Nightly `live-smoke` job drives released-artifact code against real Yahoo/Binance/CoinGecko/SEC/ECB + an MCP stdio E2E ([nightly.yml](../../.github/workflows/nightly.yml)). This testing found and fixed real rot: Yahoo's crumb requirement, SEC CIK padding, post-ASC-606 revenue concepts. |
| 2 | **Awkward-API correctness** — quote/options/fundamentals, not just daily bars | The easy 80% is a yfinance call; agents need the rest. | Yahoo cookie+crumb handshake (lazy, on 401/403 only); SEC XBRL revenue-concept fallback chain; CIK normalization — each with live tests. |
| 3 | **Bounded I/O by construction** | A stalled upstream must not hang your agent's tool call indefinitely. | All provider HTTP flows through one client factory with 10s connect / 30s request timeouts (`tdw_core::http_support::build_client`); tool-exec output is capped; wire-boundary input validation on every op. |
| 4 | **Uniform multi-asset surface** | One schema'd call shape across equities/FX/crypto/filings beats N bespoke tools. | `tdw.provider.fetch` dispatches every compiled-in `(provider, endpoint)` — 34 providers / 51 endpoints — with a registry drift-guard test; see the [demo session](mcp-demo-session.md) crossing 4 asset classes in one session. |
| 5 | **Catalog honesty** | Demo/debug tools in `tools/list` waste agent context and invite misuse. | `*.sample` evidence tools hidden by default (still callable for smokes); server discloses fixture-vs-live mode in `initialize`. |
| 6 | **A durability story** — what happens when you outgrow "fetch on demand"? | Research loops eventually want history, replay, lineage. | The same provider registry feeds `tdw-service`/`tdw-worker`: event-sourced ingestion into Postgres/ClickHouse/Qdrant/S3, with [cross-backend conformance suites](../../crates/tdw-worker/tests/conformance.rs) guaranteeing in-memory prototypes behave identically on durable backends. |
| 7 | **Supply-chain posture** | You're piping this into capital decisions. | Tagged releases ship Linux/macOS/Windows binaries + GHCR images with build-provenance attestations; MIT/Apache-2.0; per-crate pedantic lint walls and perf-regression ratchets in CI. |
| 8 | **Production auth** | Remote MCP without real auth is an incident. | Fail-closed daemon; structural + cryptographic JWT verification (`alg:none`/HMAC-confusion rejected); [IdP setup guide](../release/oidc-idp-setup.md). |
| 9 | **Honest failure modes** | Agents act on errors; silent fallbacks poison decisions. | Missing keys/failed fetches surface as structured `isError` tool results, never canned data; durability-sink failures are counted and scrapeable, not swallowed. |
| 10 | **Reproducible evaluation** | You shouldn't have to trust this table. | [MCP quickstart](mcp-quickstart.md) (<10 min) and the [15-minute warehouse eval](warehouse-install.md) are checkpoint-gated; the [demo transcript](mcp-demo-session.md) is replayable verbatim. |

## How to use this against alternatives

For each candidate server ask: where is the nightly live test? what happens on
a Yahoo 401 or an SEC concept rename? what bounds a hung upstream? how many
endpoints are reachable through how many tool shapes? what does `tools/list`
show an agent? where does the data go when you need it tomorrow? who signs the
artifact you run?

A wrapper around a single Python library typically answers: none, breakage,
nothing, few-through-many, everything, nowhere, nobody. That gap — not feature
count — is the difference between a demo and infrastructure.

*Claims above are pinned to this repository at the linked sources; re-verify
them in your own checkout rather than trusting prose.*
