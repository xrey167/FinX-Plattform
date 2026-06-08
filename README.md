# FinX-Plattform

A Rust workspace for a trading data warehouse — event-sourced, provider-agnostic, designed to make market data, agent workflows, and storage layers composable without locking into any one vendor.

**Status:** stable 1.x release line. The latest published 1.x tag is tracked in
[`CHANGELOG.md`](CHANGELOG.md); APIs should now change only through explicit
migration notes and SemVer-compatible release planning.

---

## What this is

FinX-Plattform (`tdw-*`) is a clean-room Rust workspace that splits a trading data warehouse into ~80 small crates organized around a shared event spine. Each crate has a single responsibility and is independently testable; nothing depends on a specific database, LLM vendor, or market data provider at the contract layer.

Concretely the workspace contains:

- **Core contracts** — `tdw-event`, `tdw-session`, `tdw-rollout`, `tdw-snapshot`, `tdw-bus`, `tdw-outbox`, `tdw-cdc`, `tdw-domain`, `tdw-protocol`.
- **Storage backends** — Postgres, ClickHouse, S3/MinIO, Qdrant, Meilisearch, Parquet, plain filesystem, plus a router that fans queries across them.
- **Market data providers** — Yahoo Finance, FRED, Polygon, Alpaca, Binance, HuggingFace, fileset, and a WebSocket mock for tests.
- **LLM + embeddings** — Anthropic and OpenAI-compatible adapters, local / OpenAI / Google embedding backends, behind a single `tdw-llm` interface.
- **Agents + tooling** — agent registry, hooks, sandboxed UDFs (JS / Python / WASM / external), MCP and ACP servers, a TUI, a CLI.
- **Pipelines + SQL** — staging, dbt runner, migrations, SQL codegen, table-format adapters.

The full crate roster and per-crate audit state lives in [`docs/quality/crate-readiness/matrix.md`](docs/quality/crate-readiness/matrix.md).


The boundary is enforced by `cargo run -p xtask -- clean-room-audit`, which runs in CI on every PR.

## Architecture

Work is organized into tranches (`G0NN-<topic>`) that map to active worktrees on `work/<topic>` branches. A given tranche typically lands a coherent slice of functionality — e.g., G010 brings up storage adapters, G011 brings up market data providers, G013 wires the event spine end-to-end.

```
┌─────────────────────────────────────────────────────────────────┐
│  clients         tdw-cli   tdw-tui   tdw-app-client   tdw-mcp   │
├─────────────────────────────────────────────────────────────────┤
│  service         tdw-service   tdw-service-api   tdw-app-server │
├─────────────────────────────────────────────────────────────────┤
│  agents/tools    tdw-agent  tdw-hooks  tdw-tools  tdw-sandbox   │
│                  tdw-udf-{js,python,wasm,external}              │
├─────────────────────────────────────────────────────────────────┤
│  knowledge/llm   tdw-knowledge  tdw-kg  tdw-tags  tdw-llm-*     │
│                  tdw-embed-{local,openai,google}                │
├─────────────────────────────────────────────────────────────────┤
│  pipelines       tdw-pipe  tdw-stage  tdw-dbt-runner            │
├─────────────────────────────────────────────────────────────────┤
│  event spine     tdw-event  tdw-bus  tdw-outbox  tdw-cdc        │
│                  tdw-session  tdw-snapshot  tdw-rollout         │
├─────────────────────────────────────────────────────────────────┤
│  storage         tdw-storage-{postgres,clickhouse,s3,qdrant,    │
│                  meilisearch,parquet,fs,router}                 │
├─────────────────────────────────────────────────────────────────┤
│  providers       tdw-provider-{yahoo,fred,polygon,alpaca,       │
│                  binance,huggingface,fileset,ws-mock}           │
└─────────────────────────────────────────────────────────────────┘
```

## Security / production auth

The `tdw-service` daemon is fail-closed by default; a `prod`/`production` profile
attaches an auth-backed ingress policy from the six `TDW_OIDC_*` variables
(structural claim/JWKS validation, not cryptographic signatures). See
[`docs/release/production-auth-oidc.md`](docs/release/production-auth-oidc.md).

## Quickstart

Prerequisites: Rust toolchain pinned by [`rust-toolchain.toml`](rust-toolchain.toml) (currently 1.95.0). Docker Desktop is only needed for live container smoke tests; the offline gates run without it.

```powershell
git clone https://github.com/xrey167/FinX-Plattform.git
cd FinX-Plattform

cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo run -p xtask -- clean-room-audit
```

Local infra (Postgres, ClickHouse, optionally Qdrant / Meilisearch / MinIO / Redis) comes up via Docker Compose profiles — see [`docs/docker.md`](docs/docker.md). Before the first `live` bring-up, run the idempotent setup helper to create `.env` and a random MCP token:

```powershell
.\scripts\compose-setup.ps1   # or: ./scripts/compose-setup.sh
```

Every environment variable is documented in [`docs/CONFIGURATION.md`](docs/CONFIGURATION.md).

## Repository layout

```
FinX-Plattform/
├── crates/                  # ~80 tdw-* crates (one responsibility each)
├── xtask/                   # workspace automation (audits, codegen, lint)
├── docs/
│   ├── docker.md            # local compose + WSL2 guidance
│   └── quality/             # readiness matrix + per-crate audit notes
├── scripts/
│   ├── git/new-worktree.ps1 # create sibling worktree on work/<topic>
│   └── github/              # remote setup helpers
├── .github/workflows/       # ci.yml + codeql.yml
├── AGENTS.md                # operational rules (branches, PRs, verification)
└── CONTRIBUTING.md          # contributor onramp
```

The workspace folder (`FinX-Finance/`) one level up holds parallel-phase worktrees as siblings; see its `AGENTS.md` for the multi-worktree layout.

## Contributing

External contributions are welcome — bug reports, fixes, providers, storage adapters. The basics:

1. Read [`AGENTS.md`](AGENTS.md) for branch / commit / PR conventions and [`CONTRIBUTING.md`](CONTRIBUTING.md) for the local checks.
2. Open an issue before non-trivial work so we can agree on scope.
3. Branch off `main` as `work/<topic>` (or `fix/<topic>`, `docs/<topic>`, `chore/<topic>`).
4. Run the four verification commands before opening a PR. Squash-and-merge only.
5. Keep the clean-room boundary intact: no `finx-*` imports, nothing copied from FinX-XR, no `tdw-provider-openbb`.

See the License section below for how contributions are licensed.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

`Cargo.toml` still keeps `publish = false` on every crate — the `tdw-*` crates are not on crates.io yet. The dual license applies to the source as it lives in this repository.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
