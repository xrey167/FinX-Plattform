# tdw-mcp tool-surface audit (real MCP client)

<!-- P1.3 deliverable. Audited 2026-06-09 against `tdw-mcp --stdio-json-rpc`
     (debug build, main @ 30879dbf) by driving a raw JSON-RPC session the way
     Claude Code / any MCP client does: initialize → notifications/initialized
     → tools/list → resources/list → prompts/list → tools/call. -->

## Verdict

The protocol surface is solid; the **product** surface was not: an offline
build silently serves canned market data under real provider names. The new
`live` feature on `tdw-mcp` (this PR) closes that gap — it forwards to
`tdw-service-api/all-http-providers`, swapping the Yahoo fixture for the real
HTTP fetcher and registering every live HTTP provider. Distribution builds
(GHCR images, release binaries) MUST enable `--features live`.

## What a client sees (audited)

| Surface | Result |
|---|---|
| `initialize` | Correct: echoes requested `protocolVersion` (tested 2025-06-18), declares tools/resources/prompts capabilities, returns `serverInfo` + `instructions`. |
| `tools/list` | 10 built-in tools, every one with JSON input schema and MCP annotations (`readOnlyHint` etc.). |
| `resources/list` | 4 `tdw://` resources (quality docs, protocol config sample, capabilities). |
| `prompts/list` | 3 prompts (`tdw.equity.research`, `tdw.daemon.triage`, `tdw.ingest.plan`). |
| `tools/call` | Works; returns both `content[].text` and `structuredContent`. Unknown method → `-32601`. |

### Tool inventory

| Tool | Kind | Notes |
|---|---|---|
| `tdw.providers.list` | data, read-only | Registry contents; offline build lists only fileset/yahoo/mock-ws. |
| `tdw.equity.historical` | data, read-only | The flagship data tool. **Offline build returns deterministic fixture bars (close=202.0, 2026-05-21) for `provider=yahoo` — indistinguishable from real data to a client.** With `--features live`, dispatches the real Yahoo HTTP fetcher. |
| `tdw.progress.sample`, `tdw.agent.sample`, `tdw.extensibility.sample`, `tdw.event_spine.sample`, `tdw.kg_tag.sample`, `tdw.client_event.sample` | evidence samples | Deterministic; fine as-is, but 6 of 10 tools being samples dilutes the catalog a financial-data client sees. Candidate: hide behind an env/feature for distribution builds (follow-up, not this PR). |
| `tdw.daemon.triage`, `tdw.daemon.query.submit` | daemon-backed | Require a configured daemon; described truthfully. |

## Provider API-key config story (audited from code)

- Keyless providers need nothing: yahoo, ecb, sec, binance, coingecko
  (coingecko optionally forwards `COINGECKO_API_KEY` when set).
- FRED requires `FRED_API_KEY` (free key) even for public series.
- Keyed providers (polygon, alpaca, alpha-vantage, fmp, tiingo, …) read their
  keys from per-provider env vars at fetch time; absent keys surface as
  provider errors in `tools/call` results (`isError: true`), not transport
  failures — correct MCP behavior.
- There is no key *validation* at startup: a misconfigured key is only
  discovered on first call. Acceptable for v1.2; quickstart (P1.4) must state
  which env vars each provider reads.

## Verification

- Offline build: full audit session transcript replayed; fixture bars
  confirmed for `yahoo`/`AAPL` (close 202.0 dated 2026-05-21).
- `--features live` build: same session returns real Yahoo chart bars for
  `AAPL` and `tdw.providers.list` grows to the full HTTP provider set.
- Existing offline tests are unaffected: the feature is off by default and
  `default_registry()` keeps its 3 offline providers without it.
