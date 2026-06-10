# Launch announcement — draft (P1.8, human-gated)

<!-- DRAFT prepared by the go-live loop. PUBLICATION IS HUMAN-ONLY.
     Blocked on DECISIONS.md D2. Tone: factual, builder-to-builder. -->

## Short form (registry blurb / social)

> **tdw-mcp v1.2.0** — live market data in Claude (or any MCP client).
> One docker command: equity bars, SEC filings, FX, crypto — 34 providers,
> keyless tier works instantly, your API keys stay in your environment.
> Rust, self-hostable, open source (MIT/Apache-2.0).
>
> `docker run -i --rm ghcr.io/xrey167/finx-plattform-tdw-mcp:latest --stdio-json-rpc`

## Long form (blog / Discussions / HN "Show" post)

**Show: tdw-mcp — an open-source MCP server for live market data**

We built a trading data warehouse in Rust (58 crates, clean-room) and put an
MCP server in front of it. v1.2.0 is the first release where the whole path is
verified live: a real MCP client fetching real AAPL bars, real SEC filings,
real crypto OHLC — not fixtures.

What you get:
- **10-minute setup**: public GHCR image or single binary; quickstart walks
  Claude Code and Claude Desktop wiring (`docs/products/mcp-quickstart.md`).
- **34 providers registered, generically dispatchable** (`tdw.provider.fetch`):
  Yahoo, SEC EDGAR, ECB, Binance, CoinGecko free-tier keyless; FRED, Polygon,
  Alpaca, Tiingo and more with your own env-var keys. Keys never leave your
  machine — the server has no cloud component.
- **Honest by construction**: offline builds disclose they serve fixtures;
  hidden demo tools stay out of the catalog; every release binary/image is
  built with live transports, Trivy-scanned, provenance-attested.
- **Self-hostable warehouse underneath** when you outgrow the single server:
  Postgres/ClickHouse/Qdrant compose stack with pinned images, healthchecks,
  backup/restore + upgrade runbooks.

Things we fixed along the way that might interest API archeologists: Yahoo's
cookie+crumb handshake (and its bot-UA 429s), SEC's zero-padded CIKs and the
post-ASC-606 revenue concept fallback chain, Binance's HTTP 451 geo-blocks on
CI runners.

Repo: https://github.com/xrey167/FinX-Plattform — quickstart, security policy,
and the full release notes (v1.2.0) are in the repo. Feedback and provider
requests welcome via Issues<!-- + Discussions if D6 enables it -->.

<!-- PRICING PARAGRAPH — insert per D2 outcome. Recommended (memo): "tdw-mcp
     is and stays open source. We're building a hosted tier (free + pro) for
     teams who don't want to run it themselves — waitlist: <link>." -->
