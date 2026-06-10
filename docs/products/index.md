# FinX-Plattform — live financial data for AI agents, with a warehouse behind it

**tdw-mcp** is an MCP server that gives Claude (or any MCP client) live market
data across asset classes — equities, FX/macro, crypto, SEC filings — backed by
an event-sourced trading data warehouse you can self-host when you outgrow the
front door.

```
┌──────────────┐   MCP (stdio / HTTP)   ┌─────────┐   live APIs   ┌───────────────┐
│ Claude / any │ ─────────────────────► │ tdw-mcp │ ────────────► │ 34 providers   │
│  MCP client  │ ◄───────────────────── │         │ ◄──────────── │ 51 endpoints   │
└──────────────┘    structured rows     └─────────┘               └───────────────┘
```

## Start in two minutes

Follow the **[quickstart](mcp-quickstart.md)** — GHCR one-liner or a released
binary, plus the Claude Desktop/Code config snippet. No API keys needed for
Yahoo, Binance, CoinGecko, SEC, or ECB; a [key table](mcp-quickstart.md#3-provider-api-keys)
unlocks the rest.

See it working first: the **[demo session](mcp-demo-session.md)** — an
annotated transcript of a real session pulling that week's AAPL bars, EUR/USD
fixings, BTC OHLC, and Apple's filings through two tools.

## Why teams pick this over a generic finance MCP

- **Live and proven live.** A nightly CI job drives the released artifact
  against the real provider APIs — including the awkward parts other servers
  skip (Yahoo's cookie+crumb handshake, SEC's CIK and post-ASC-606 revenue
  quirks were all found and fixed by live testing, not fixtures).
- **Bounded by construction.** Every provider call runs through a shared HTTP
  client with hard connect/request timeouts; tool execution output is capped;
  inputs are validated at the wire boundary.
- **One tool shape, every provider.** `tdw.provider.fetch` dispatches any
  compiled-in `(provider, endpoint)` pair with a completeness drift-guard test
  — when a provider lands, agents can call it.
- **A real warehouse behind the front door.** The same provider registry feeds
  `tdw-service` and `tdw-worker` for durable, event-sourced ingestion into
  Postgres/ClickHouse/Qdrant/S3 — with cross-backend conformance suites
  guaranteeing in-memory prototypes behave identically when promoted to
  durable storage.
- **Engineering you can audit.** ~115 single-responsibility crates, per-crate
  pedantic lint walls, performance-regression ratchets, MIT/Apache-2.0.

## The platform, beyond MCP

| Surface | What it gives you | Docs |
|---|---|---|
| `tdw-mcp` | Live financial data tools for AI agents | [quickstart](mcp-quickstart.md) |
| OpenBB parity (REST + Workspace + copilot) | Catalog-derived `GET /api/v1` + OpenAPI, OpenBB Workspace data backend + copilot | [openbb-parity](openbb-parity.md) |
| `tdw-service` + `tdw-worker` | Self-hosted, event-sourced ingestion warehouse | [local stack runbook](../release/local-stack-runbook.md) |
| `tdw-cli` / `tdw-tui` | Operator tooling | [release docs](../release.md) |
| `tdw-backend` | Embeddable Rust facade (roadmap: crates.io) | [architecture](../architecture.md) |

Releases ship Linux/macOS/Windows binaries and GHCR images with build
provenance attestations — see [release notes](../release/v1.2.0-notes.md) and
the [CHANGELOG](../../CHANGELOG.md).
