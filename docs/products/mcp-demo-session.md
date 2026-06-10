# tdw-mcp demo: a real session, live data, four asset classes

This is an annotated transcript of an actual MCP session against a `live`-built
`tdw-mcp` (stdio transport, protocol `2025-06-18`), captured 2026-06-10 during
release verification. Every response below is real API data — nothing canned.
Reproduce it with the [quickstart](mcp-quickstart.md); no API keys are needed
for anything shown here.

## 1. Handshake

```jsonc
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{
    "protocolVersion":"2025-03-26","capabilities":{},
    "clientInfo":{"name":"demo","version":"0.1"}}}
← {"serverInfo":{"name":"tdw-mcp","title":"TDW MCP Server","version":"0.1.0"},
   "protocolVersion":"2025-06-18", ...}
```

`tools/list` returns a catalog that leads with data and daemon tools —
`tdw.providers.list`, `tdw.equity.historical`, `tdw.provider.fetch`,
`tdw.daemon.*` (demo/evidence tools are hidden unless
`TDW_MCP_SAMPLE_TOOLS=1`).

## 2. Live equity bars (Yahoo)

```jsonc
→ {"method":"tools/call","params":{"name":"tdw.equity.historical",
    "arguments":{"symbol":"AAPL","provider":"yahoo"}}}
← rows: [
    {"date":"2026-06-03","open":314.17,"high":316.94,"low":308.85,
     "close":310.26,"volume":50836700,"symbol":"AAPL"},
    {"date":"2026-06-04","close":311.23, ...},
    ... 5 trading days
  ]
```

That week's actual AAPL OHLCV — fetched live at call time through the bounded
provider HTTP client (10s connect / 30s request timeouts on every provider).

## 3. One generic tool, every compiled-in provider

`tdw.provider.fetch` dispatches any `(provider, endpoint)` pair the build
registers — 34 providers / 51 fetcher endpoints in the `live` build:

**FX/macro — ECB euro reference rates:**

```jsonc
→ {"name":"tdw.provider.fetch","arguments":{"provider":"ecb","endpoint":"data",
    "params":{"flow":"EXR","key":"D.USD.EUR.SP00.A",
              "start_period":"2026-06-01","end_period":"2026-06-09"}}}
← rows: 7 daily USD/EUR observations, e.g. {"date":"2026-06-01","flow":"EXR",...}
```

**Crypto — CoinGecko BTC OHLC (7 days):**

```jsonc
→ {"name":"tdw.provider.fetch","arguments":{"provider":"coingecko",
    "endpoint":"ohlc","params":{"coin_id":"bitcoin","vs_currency":"usd","days":7}}}
← rows: 42 four-hourly bars, latest close ≈ 66,650 USD
```

**Regulatory — SEC EDGAR filings for Apple (CIK 320193):**

```jsonc
→ {"name":"tdw.provider.fetch","arguments":{"provider":"sec",
    "endpoint":"filings","params":{"cik":"320193"}}}
← rows: 1,000 filings, newest first, each {"accession_number":...,"form":"10-K"|...}
```

Equities, FX/macro, crypto, and regulatory filings — one server, one tool
shape, zero configuration, in a single session.

## Why this is hard to get elsewhere

- **Live, not canned**: the same binary your agent runs in production answered
  these calls; the nightly `live-smoke` CI job replays this exact flow against
  the real APIs so regressions surface within a day.
- **Verified conformance**: results flow through provider transports that are
  live-tested per provider (including Yahoo's cookie+crumb handshake for
  quote/profile/options, SEC CIK normalization, and post-ASC-606 revenue
  concept resolution — all bugs found and fixed by testing against the real
  APIs, not fixtures).
- **A warehouse behind it**: the same registry feeds `tdw-service`/`tdw-worker`
  for durable ingestion — the MCP server is the front door, not the whole
  house.

Keys unlock more (FRED macro, Polygon, Alpaca, Tiingo, FMP, …) — see the
[provider API-key table](mcp-quickstart.md#3-provider-api-keys).
