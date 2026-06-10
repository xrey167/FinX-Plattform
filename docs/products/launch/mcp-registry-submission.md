# MCP registry submission — draft (P1.8, human-gated)

<!-- DRAFT prepared by the go-live loop. SUBMISSION IS HUMAN-ONLY per the
     ledger guardrail. Blocked on DECISIONS.md D2 (licensing) being answered.
     Verify the registry's current schema at submission time:
     https://github.com/modelcontextprotocol/registry -->

## server.json (draft)

```json
{
  "name": "io.github.xrey167/tdw-mcp",
  "description": "Live market data over MCP: equity bars, 34 financial-data providers (Yahoo, SEC EDGAR, ECB, Binance, CoinGecko, FRED, Polygon, ...), generic provider dispatch, research prompts. Rust, self-hostable, MIT OR Apache-2.0.",
  "version": "1.2.0",
  "repository": {
    "url": "https://github.com/xrey167/FinX-Plattform",
    "source": "github"
  },
  "packages": [
    {
      "registryType": "oci",
      "identifier": "ghcr.io/xrey167/finx-plattform-tdw-mcp",
      "version": "1.2.0",
      "transport": { "type": "stdio" },
      "runtimeArguments": ["--stdio-json-rpc"]
    }
  ]
}
```

Notes for the submitter:
- The OCI image is public and multi-arch (amd64+arm64), built `--features live`,
  Trivy-scanned, provenance-attested. `latest` and `sha-<commit>` tags exist;
  pin the release sha or publish a `v1.2.0` image tag first if the registry
  requires an immutable version reference (one-line addition to ci.yml tags).
- Binaries alternative: the v1.2.0 GitHub release carries signed-checksum
  archives for 4 platforms if a `binary` package entry is preferred.
- Env vars (optional, per provider): see the API-key table in
  [mcp-quickstart.md](../mcp-quickstart.md). Keyless default works out of the box.

## Submission steps (human)

1. Confirm D2 (licensing) is answered — the description above states
   MIT OR Apache-2.0; adjust if D2 changes the model.
2. Validate `server.json` against the registry's current schema
   (`mcp-publisher` CLI or the registry repo's docs — the schema has churned;
   do not trust this draft's field names blindly).
3. Submit via the registry's publish flow (GitHub auth ties the
   `io.github.xrey167` namespace to the repo owner).
4. After acceptance, smoke-test discovery from a fresh Claude client.
