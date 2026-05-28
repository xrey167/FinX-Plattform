# Daemon Hardening Test Taxonomy

Date: 2026-05-28

This taxonomy keeps daemon, MCP, worker, provider, and backend tests in the
right buckets. The goal is to preserve a fast offline signal while making real
backend and live-network coverage explicit.

## Always-on unit and contract tests

Run on every local/CI pass without secrets or Docker:

- `cargo test --workspace --no-default-features`
- `cargo test -p tdw-service-api`
- `cargo test -p tdw-app-server`
- `cargo test -p tdw-mcp -p tdw-worker`
- `cargo test -p tdw-sandbox -p tdw-udf -p tdw-udf-wasm`

Covered behavior:

- AppState local policy synthesis and production fail-closed behavior.
- Dispatcher auth/hook/mask/UDF enforcement.
- ServiceLoop persistence and cost recording.
- MCP JSON-RPC stdio framing contract.
- Worker queue contract semantics.
- UDF name/source/input/capability guards and Wasm fixture validation.

## Docker or real-backend integration tests

Run only when a real backend is available. Tests must skip cleanly when the
required environment variables are unset.

- `just test-e2e-pg`
- `cargo test --package tdw-service --test g0xx_daemon_e2e_real_pg -- --nocapture`

Required environment:

- `TDW_POSTGRES_TEST_URL`
- Optional full profile URLs for ClickHouse, S3/MinIO, Qdrant, and Meilisearch
  when a broader profile is added.

Expected setup:

- `docker compose --profile full up -d`

## Live-network provider and model tests

Run only with explicit credentials and live-network opt-ins. These tests must
not run in the default workspace gate.

- Provider tests: Alpaca, Binance, FRED, HuggingFace, Polygon, Yahoo live modes.
- LLM and embedding tests: Anthropic, OpenAI-compatible, OpenAI embeddings, and
  Google embeddings live modes.

Each live test must have:

- A deterministic cassette/offline test.
- A provider-specific env-var gate.
- Clear failure evidence when credentials are missing or rejected.

## Final quality gate

Before treating daemon-hardening work as complete, run:

- `cargo +stable fmt --all -- --check`
- `cargo +stable check --workspace`
- `cargo +stable clippy --workspace --all-targets -- -D warnings`
- `cargo +stable test --workspace`
- `cargo +stable run -p xtask -- schema-sync`
- `cargo +stable run -p xtask -- events schema-check`
- `cargo +stable run -p xtask -- protocol schema-check`
- `cargo +stable run -p xtask -- quality-gate check`
- `cargo +stable run -p xtask -- clean-room-audit`
- `git diff --check`

On this Windows workstation, use `CARGO_TARGET_DIR` under `%TEMP%` when the
shared `E:\cargo-target` directory reports access-denied or stale artifact
errors.
