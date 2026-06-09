# tdw-mcp quickstart — live market data in your MCP client

<!-- P1.4 deliverable. Every command here is verified against the build it
     documents; the GHCR path additionally requires the first green
     Container Image run on main (see "Image availability" below). -->

tdw-mcp is an MCP (Model Context Protocol) server that serves market data —
equity bars, provider registry, research prompts — to Claude Code, Claude
Desktop, or any MCP client, over stdio or Streamable HTTP.

## 1. Run it

### Option A — Docker (GHCR)

```sh
docker run -i --rm ghcr.io/xrey167/finx-plattform-tdw-mcp:latest --stdio-json-rpc
```

The image entrypoint is the `tdw-mcp` binary; arguments pass through. Images
are multi-arch (amd64 + arm64), built with the `live` feature, scanned with
Trivy, and provenance-attested. Tags: `latest` and `sha-<commit>`.

### Option B — build from source

Requires the Rust toolchain pinned in `rust-toolchain.toml` (1.95.0).

```sh
cargo build --release -p tdw-mcp --features live --bin tdw-mcp
./target/release/tdw-mcp --stdio-json-rpc
```

> **The `live` feature matters.** Without it the server still runs, but
> `tdw.equity.historical` answers with a deterministic offline fixture —
> useful for tests, wrong for real use. Distribution images enable it.

### Transports

| Flag | Transport |
|---|---|
| `--stdio-json-rpc` | JSON-RPC over stdin/stdout (what MCP clients spawn) |
| `--streamable-http [bind]` | MCP Streamable HTTP; bind defaults to `127.0.0.1:8788` — pass e.g. `0.0.0.0:8788` to override |

Set `TDW_MCP_OPS_BIND` to additionally expose `/health` and `/ready` probes
for compose/K8s healthchecks.

## 2. Wire it into Claude

**Claude Code** — one command:

```sh
claude mcp add tdw -- docker run -i --rm ghcr.io/xrey167/finx-plattform-tdw-mcp:latest --stdio-json-rpc
```

**Claude Desktop** — edit `claude_desktop_config.json`
(macOS: `~/Library/Application Support/Claude/`, Windows: `%APPDATA%\Claude\`),
then restart Claude Desktop:

```json
{
  "mcpServers": {
    "tdw": {
      "command": "docker",
      "args": ["run", "-i", "--rm",
               "-e", "FRED_API_KEY",
               "ghcr.io/xrey167/finx-plattform-tdw-mcp:latest",
               "--stdio-json-rpc"],
      "env": { "FRED_API_KEY": "<your key>" }
    }
  }
}
```

GUI apps don't inherit your shell environment — put key values in the `env`
block (it sets them for the spawned `docker` process) and pass-through
`-e VAR` flags forward them into the container, one per key from the table
below. For a from-source install, set `command` to the built binary path and
`args` to `["--stdio-json-rpc"]`; keys then come straight from `env`.

## 3. Provider API keys

Keys are read from environment variables at fetch time. No key is validated
at startup — a missing/wrong key surfaces as an `isError: true` tool result
on first use.

> **Routable today:** `tdw.equity.historical` currently dispatches `yahoo`
> (keyless) and `fileset`; the other providers below are registered and
> listed by `tdw.providers.list`, and become callable when generic dispatch
> lands (P1.3b). Configure their keys now or later — nothing breaks either
> way.

**No key needed:** yahoo, sec, ecb, binance, coingecko (free tier).

| Provider | Env var | Notes |
|---|---|---|
| fred | `FRED_API_KEY` | Free: https://fred.stlouisfed.org/docs/api/api_key.html |
| coingecko | `COINGECKO_API_KEY` | Optional; raises free-tier rate limits |
| polygon | `POLYGON_API_KEY` | |
| alpaca | `APCA_API_KEY_ID` + `APCA_API_SECRET_KEY` | Both required |
| alpha-vantage | `TDW_ALPHA_VANTAGE_API_KEY` | |
| fmp | `TDW_FMP_API_KEY` | |
| tiingo | `TDW_TIINGO_API_KEY` | |
| finnhub | `TDW_FINNHUB_API_KEY` | |
| nasdaq | `TDW_NASDAQ_API_KEY` | |
| databento | `TDW_DATABENTO_API_KEY` | |
| benzinga / bls / ccdata / eia / glassnode / tradier / trading-economics / velodata / adanos | `TDW_<PROVIDER>_API_KEY` | Same pattern, uppercased provider name |
| seeking-alpha | `TDW_SEEKING_ALPHA_API_KEY` | RapidAPI key |
| huggingface | `HF_TOKEN` (or `HUGGINGFACE_API_TOKEN` / `HF_API_TOKEN`) | |

## 4. First query

Ask Claude: *"Use the tdw server to fetch recent AAPL bars from yahoo."*
Claude calls:

```json
{"method": "tools/call", "params": {"name": "tdw.equity.historical",
 "arguments": {"provider": "yahoo", "symbol": "AAPL"}}}
```

and receives real OHLCV rows in `structuredContent`. To see everything the
server offers, list its tools (10), resources (4 `tdw://` docs), and prompts
(3 guided workflows — `tdw.equity.research`, `tdw.daemon.triage`,
`tdw.ingest.plan`).

Current dispatch note: `tdw.equity.historical` currently routes `yahoo` and
`fileset`; the remaining registered live providers are exposed via
`tdw.providers.list` and become callable as generic dispatch lands (tracked
as P1.3b). The full tool-surface audit lives in
[mcp-tool-surface-audit.md](mcp-tool-surface-audit.md).

## Image availability

GHCR images publish on every green `Container Image` run on `main`
(`.github/workflows/ci.yml`, `images` job). If `docker pull` returns
`manifest unknown`, no green main run has happened since the last
image-affecting change — build from source (Option B) until it goes green.
