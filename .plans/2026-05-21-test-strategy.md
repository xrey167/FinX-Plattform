# FinX-Finance — Test Strategy (Layer F)

**Project:** FinX-Finance (`C:\Users\ReyDa\FinX-Finance\`)
**Date:** 2026-05-21
**Mode:** `/omc-plan --direct` (transverse layer; modifies every phase, adds Phase 0.5 + Phase 15)
**Status:** Draft — quality contract that all other plans must meet
**Parent plans:**
- [`2026-05-21-rust-trading-data-warehouse.md`](./2026-05-21-rust-trading-data-warehouse.md) — core (Phases 0–6)
- [`2026-05-21-data-engineering-and-agent-schemas.md`](./2026-05-21-data-engineering-and-agent-schemas.md) — Layer A+B (Phases 7–8)
- [`2026-05-21-hook-event-spine.md`](./2026-05-21-hook-event-spine.md) — Layer E (Phase 9)
- [`2026-05-21-databend-surrealdb-feature-parity.md`](./2026-05-21-databend-surrealdb-feature-parity.md) — Layer C (Phases 10–14)
- [`2026-05-21-connect-rust-buffa-evaluation.md`](./2026-05-21-connect-rust-buffa-evaluation.md) — evaluation only

---

## 1. Goal

Define the test strategy that ensures FinX-Finance is **production-quality on day one for a single user**, not "production-quality once you scale a team":

1. **≥ 80% line coverage** per crate, measured by `cargo-llvm-cov`, gated in CI. Critical-path crates (`tdw-auth`, `tdw-mask`, `tdw-hooks`, `tdw-outbox`, `tdw-snapshot`) target **≥ 95%**.
2. **Six test tiers**, used deliberately: unit · integration · property · e2e · mutation · bench. Each tier has a defined cost/value role; none replaces another.
3. **Test infrastructure ships in Phase 0.5** (new), so every subsequent phase has the right scaffolding from day one. No "we'll add tests later."
4. **Per-phase test deliverables** are part of the phase's exit criteria, not bolt-on work.
5. **CI gates** block merge on coverage regression, integration failure, or schema/spec drift.
6. **Phase 15 (new) — Hardening + E2E flows** catches cross-phase scenarios that single-phase tests can't reach (full ingest → transform → embed → search → MCP roundtrip).

This plan does not invent new functionality. It binds every other plan to a **measurable quality contract**.

---

## 2. RALPLAN-DR Summary

### Principles
1. **The test pyramid is real, not a slogan.** ~70% unit (fast, no infra) · ~20% integration (infra via `testcontainers`) · ~5% property (proptest invariants) · ~3% e2e (full stack docker-compose) · ~2% mutation (`cargo-mutants` nightly). Bench runs are gates, not coverage units.
2. **Tests are an invariant, not a phase.** Every PR ships passing tests + ≥80% coverage delta. There is no "I'll add tests next sprint" — CI blocks the merge.
3. **Test infrastructure is built once, used everywhere.** A shared `tdw-test-utils` crate provides `testcontainers` fixtures, `wiremock` provider tapes, golden-file harness, property-test generators, and bench harness. Per-phase tests reuse this; no duplication.
4. **Adversarial tests are first-class.** Sandbox escapes (UDF), recursive event cycles (hooks), permission bypasses (masking), schema drift (DDL codegen), retry exhaustion (outbox) all have explicit adversarial test suites, not just happy-path coverage.
5. **Coverage > 80% is the floor, not the goal.** Mutation score (do tests catch deliberate bugs?) is the truer signal; we measure both and treat low mutation score as a failed test.

### Decision Drivers (top 3)
1. Personal scope means no QA team to backstop missing tests — the test suite *is* the reviewer.
2. ~47-crate workspace means a "smoke-test everything by running it" approach doesn't scale. Tests must be automatable + fast.
3. Foundational crates (spine, outbox, masking, auth) carry every other crate's correctness — high coverage on them prevents cascade failures.

### Viable Options

#### Option A — Six-tier pyramid with shared test utilities + per-phase gates *(chosen)*
- **Pros**: explicit cost/value per tier; shared infra avoids duplication; gates prevent regression; mutation testing catches weak assertions.
- **Cons**: upfront infra investment (Phase 0.5 ≈ 4 days); ~25–30% of dev time goes to tests; slow tier (e2e+mutation) needs nightly CI capacity.

#### Option B — Unit + integration only, skip property/mutation/e2e
- **Pros**: cheaper to set up; familiar.
- **Cons**: misses invariant violations under concurrency, misses cascade bugs, misses cross-phase regressions. Spine (Phase 9) cannot be honestly tested without property tests (recursion, ordering); UDF sandbox (Phase 14) cannot be tested without adversarial e2e.
- **Invalidation rationale**: the spine and UDF sandbox specifically demand property + adversarial testing; falling back to unit + integration leaves the most load-bearing crates undertested.

#### Option C — Manual testing + small smoke test suite
- **Pros**: minimal setup.
- **Cons**: undetectable regression risk on a ~47-crate, ~100-day project owned by one person. Hard reject.
- **Invalidation rationale**: single-person ownership *amplifies* the need for automation, not the opposite — there's no second pair of eyes.

---

## 3. Test Taxonomy

| Tier | Tooling | Purpose | Speed | When run |
|------|---------|---------|-------|----------|
| **Unit** | `cargo test`, `cargo nextest`, `mockall`, `assert_matches`, `pretty_assertions` | Small, pure, no I/O. Tests a single function or trait impl. | < 100 ms each | Every PR (pre-merge) |
| **Integration** | `testcontainers-rs`, `wiremock`, `sqlx-test`, `axum-test` | Tests across crate boundaries with real infra (Postgres, ClickHouse, Qdrant, Meilisearch, MinIO, Redis). | 1–30 s each | Every PR (pre-merge) |
| **Property** | `proptest`, `quickcheck`, `arbitrary` | Generates random inputs to assert invariants (ordering, idempotency, serialization round-trip). | 5–60 s each | Every PR (pre-merge, shorter shrink budget) + nightly (full budget) |
| **E2E** | `docker-compose` full stack + `tdw-cli` driver scripts | Full pipeline: HTTP fetch → store → embed → search → MCP roundtrip. | 30 s – 5 min each | Pre-merge (subset) + nightly (full) |
| **Mutation** | `cargo-mutants` | Deliberately mutates source code; asserts tests catch each mutation. | 10–60 min/crate | Nightly per crate; quarterly full workspace |
| **Bench** | `criterion`, `iai-callgrind` | Performance regression detection. Not coverage; a gate. | Variable | Nightly + pre-release; PR if changed crate is bench-tagged |

**The pyramid in numbers (target):**
- ~70% of test count = unit
- ~20% = integration
- ~5% = property
- ~3% = e2e
- ~2% = mutation
- Bench is orthogonal; ≥ 1 bench per perf-critical crate (Phase 4 hybrid search, Phase 9 spine, Phase 10 snapshot writes).

---

## 4. Test Stack (concrete crate choices)

| Concern | Crate | Rationale |
|---------|-------|-----------|
| Test runner | `cargo-nextest` | 60% faster than `cargo test`; per-test process isolation; retry-on-flake; JUnit XML output |
| Coverage | `cargo-llvm-cov` | Modern, uses LLVM source-based coverage; `--ignore-filename-regex` for generated code; emits HTML + lcov |
| Mocking | `mockall` | Trait-based mocks via `#[automock]`; works with `async-trait` |
| HTTP mocking | `wiremock-rs` | Provider tapes for `tdw-provider-*`; golden-tape regression |
| DB containers | `testcontainers-rs` (`testcontainers-modules`) | PG, ClickHouse, Redis, MinIO, Qdrant, Meilisearch all have community modules; fresh container per test by default, shared per file via `LazyLock` |
| Property testing | `proptest` (preferred over `quickcheck`) | Better shrinking, persistence (saved counterexamples in `proptest-regressions/`) |
| Snapshot testing | `insta` | Golden files for serde round-trips, generated SQL DDL, JSON Schema exports, OBBject envelopes |
| E2E driver | `tokio` + `reqwest` + `tdw-cli` + `docker-compose` | No external e2e framework; scripts live in `tests/e2e/`, run via `cargo test --test e2e --features e2e` |
| Mutation testing | `cargo-mutants` | Plain, dependency-light; mutates `+/-`, `&&/||`, `==/!=`, etc.; reports survivors |
| Bench | `criterion` for time, `iai-callgrind` for instruction count (CI-stable) | Time benches give realistic regression signal; iai-callgrind is deterministic-friendly for CI gates |
| Concurrency / fuzzing | `loom` (concurrency models for `tdw-bus`), `cargo-fuzz` for parsers (e.g. SurrealQL-lite, DEFINE schema parser) | Loom catches data races invisible to `cargo test`; fuzz catches malformed-input panics |
| Test data generation | `fake-rs`, `chrono::TimeZone` helpers | Realistic synthetic data for OHLCV, instruments, fundamentals; deterministic via seeded RNG |
| Schema drift | `insta` + custom `xtask` | Golden JSON-Schema files in `schemas/`; CI rejects drift |
| Assertions | `assert_matches`, `pretty_assertions`, `claim` | Better diff output; `assert_matches!` for enum variants |
| Test orchestration | `cargo-make` or `just` task runner | One-command flows: `just test-unit`, `just test-integration`, `just test-e2e`, `just coverage` |

---

## 5. Coverage targets + gating

### Per-crate floors

| Tier | Target | Gate |
|------|--------|------|
| Foundational (must not break) — `tdw-core`, `tdw-event`, `tdw-bus`, `tdw-hooks`, `tdw-outbox`, `tdw-snapshot`, `tdw-auth`, `tdw-mask`, `tdw-cdc`, `tdw-runtime`, `tdw-storage-router` | **≥ 95% line, ≥ 90% branch** | PR blocked on drop > 1pp |
| Storage engines — `tdw-storage-postgres`, `tdw-storage-clickhouse`, `tdw-storage-qdrant`, `tdw-storage-meilisearch`, `tdw-storage-s3`, `tdw-storage-parquet` | **≥ 85% line** | PR blocked on drop > 1pp |
| Providers — `tdw-provider-*` | **≥ 80% line** | PR blocked on drop > 2pp |
| Domain — `tdw-domain` | **100% serde round-trip** (every public struct has a golden fixture); line coverage N/A (mostly derive) | PR blocked on missing golden file |
| Tooling — `tdw-cli`, `xtask`, `tdw-test-utils` | **≥ 70% line** | PR blocked on drop > 3pp |
| Agent — `tdw-agent`, `tdw-agent-store`, `tdw-eval-runner`, `tdw-workflow-engine` | **≥ 85% line** | PR blocked on drop > 1pp |
| **Workspace aggregate** | **≥ 82% line** | Hard fail on workspace drop > 1pp |

### Mutation-score floors

| Tier | Mutation score (mutants killed / total non-trivial mutants) |
|------|----------|
| Foundational | ≥ 85% |
| Storage engines | ≥ 75% |
| Providers | ≥ 70% |
| Tooling | ≥ 60% |
| Workspace aggregate | ≥ 75% |

Mutation testing runs nightly per crate, full workspace quarterly. Survivors are triaged: real bug → fix; equivalent mutant → annotate `// MUTANT-EQUIV: reason`; acceptable miss → annotate `// MUTANT-SKIP: reason` (≤ 5% of mutants per crate without ADR).

### Performance gates

| Workload | Tooling | Gate |
|----------|---------|------|
| Equity-historical HTTP fetch p95 (cached CH read) | `criterion` + `xtask bench` | < 250 ms (parent A11) |
| ClickHouse ingest throughput (single node) | `criterion` | ≥ 50k rows/sec |
| Qdrant KNN p95 (1M points top-10) | `criterion` | < 100 ms |
| Spine event dispatch (hot lane, no hook) | `iai-callgrind` | regression > 5% fails |
| Outbox publisher latency (commit → async hook) | `criterion` | p95 < 50 ms |
| Snapshot write overhead vs no-snapshot baseline | `criterion` | < 15% (Layer C R26) |

All bench results are committed to `docs/perf-history.md` (Layer C A11); regression > 20% on rolling-7-day baseline fails CI.

---

## 6. Test Infrastructure (Phase 0.5 — NEW)

A new ~4-day phase that ships *before* Phase 1 so every subsequent phase has the right scaffolding. Inserted between Phase 0.1 (workspace skeleton) and Phase 1 (core abstractions).

### Phase 0.5 — Test Foundation — days 4–7

0.5.1. **`tdw-test-utils` crate**:
   - `containers` module: `postgres()`, `clickhouse()`, `qdrant()`, `meilisearch()`, `minio()`, `redis()` — each returns a `testcontainers::ContainerAsync<…>` and exposes `client()` / `connection_string()`. Implementations use `testcontainers-modules`.
   - `LazyContainer<T>` for per-file shared containers (init once, teardown on test-binary exit).
   - `fixtures` module: realistic OHLCV/instrument/research-note generators with seeded RNG (deterministic).
   - `golden` module: `insta`-backed snapshot helpers for serde, DDL, JSON Schema.
   - `wiremock_tapes/` directory + helper for replaying recorded HTTP responses.

0.5.2. **Property-test generators**: `proptest::strategy::Strategy` impls for `EventEnvelope`, `Actor`, `OBBject`, the 11 BOM schema types. Shrinking strategies tuned for ergonomic counterexamples.

0.5.3. **Bench harness**: `xtask bench` enumerates registered benchmarks, runs `criterion`, writes JSON to `docs/perf-history.md`. CI gate task computes regression vs rolling-7-day baseline.

0.5.4. **CI scaffolding**: GitHub Actions matrix:
   - **Job: `unit`** — Ubuntu + Windows-MSVC, debug + release; runs `cargo nextest run --workspace --no-default-features` (unit only, no infra); ≤ 2 min target.
   - **Job: `integration`** — Ubuntu; spawns `docker-compose --profile minimal up -d` + `cargo nextest run --workspace --features integration`; ≤ 15 min target.
   - **Job: `coverage`** — Ubuntu; `cargo llvm-cov nextest --workspace --lcov --output-path lcov.info`; uploads to Codecov; gates on workspace ≥ 82%.
   - **Job: `lint`** — `cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo deny check`, custom `xtask events schema-check` (Layer E V59), `xtask schema-sync` (Layer A V14).
   - **Job: `e2e-subset`** — Ubuntu; spawns full `docker-compose --profile full up -d` + runs e2e tests tagged `#[e2e(subset)]`; ≤ 20 min target.
   - **Job: `mutation`** (scheduled nightly): per-crate `cargo mutants --in-place --workspace`; reports survivors; fails if mutation-score drops below tier floor.
   - **Job: `e2e-full`** (scheduled nightly): all e2e flows; ≤ 60 min target.
   - **Job: `bench`** (scheduled nightly): runs criterion benches, commits perf-history.md update.
   - **Job: `windows-release`** — Windows-MSVC with release profile (`lto=fat`, `panic=abort`); validates `inventory` + `linkme` registration works (Layer C R6, Layer E R57).

0.5.5. **`just` task runner**: `Justfile` with `test-unit`, `test-integration`, `test-property`, `test-e2e`, `coverage`, `coverage-html`, `bench`, `bench-compare`, `mutation <crate>`, `lint`, `fmt`. Identical UX locally and in CI.

0.5.6. **Codecov configuration**: `codecov.yml` with per-crate thresholds (see §5), ignored generated code (sqlx queries, prost output if any, mockall expansions).

0.5.7. **Test naming conventions** documented in `docs/testing.md`:
   - Unit: `mod tests { #[test] fn name_describes_what() {…} }`
   - Integration: `tests/integration/<crate>_<scenario>.rs`
   - Property: `tests/property/<invariant>.rs`
   - E2E: `tests/e2e/<flow>.rs` with `#[e2e(subset)]` or `#[e2e(full)]` attribute filter
   - Bench: `benches/<workload>.rs`
   - Adversarial: `tests/adversarial/<attack_vector>.rs`

**Exit criteria**:
- `just test-unit` finishes < 2 min locally.
- `just test-integration` finishes < 15 min locally (Linux, Docker Desktop).
- `just coverage` produces an HTML report and prints workspace % ≥ a configurable floor (initially 0% — the floor ratchets up per phase).
- CI matrix is green on a no-op PR (skeleton workspace, no real code).
- `docs/testing.md` documents the six tiers + conventions.

---

## 7. Per-phase test work breakdown

Each row adds test work to the parent phase. Days listed are **test-only**, additive to the phase's existing development days.

| Phase | Test work | Test-only days | Key test types |
|-------|-----------|---------------|----------------|
| **0.0 Discovery** | Schema round-trip golden tests for the 11 re-derived BOM schemas; CI gate on YAML→Rust drift (V14). | 0.5 | unit (round-trip) |
| **0.1 Skeleton** | CI matrix green on empty workspace. | (in scope of Phase 0.5) | — |
| **0.5 Test Foundation** *(NEW)* | The phase itself — see §6. | **4** | infra |
| **1 Core abstractions** | Property tests for `Fetcher`/`Streamer` trait laws; mock implementations; OBBject serde round-trip via `insta`. | 1.5 | unit + property |
| **2 Storage engines** | Container-backed integration tests per engine; round-trip + transactional rollback; bench OLAP scan throughput. | 2 | integration + bench |
| **3 First providers** | Wiremock golden tapes for Polygon/Yahoo; offline fileset provider runs without infra; provider trait laws via property tests. | 1.5 | unit + integration |
| **4 Hybrid retrieval** | Retrieval-quality tests (golden top-10 stability vs ranked truth); Qdrant + Meili integration; embedder back-end swap test; hybrid RRF correctness. | 2.5 | integration + golden + property |
| **5 Four shells** | axum endpoint integration via `axum-test`; MCP roundtrip via stdio + HTTP-SSE; worker job-DAG ordering; CLI golden output. | 2 | integration + e2e (subset) |
| **6 Hardening** | Workspace coverage gate enabled at 80% floor; mutation-test runs introduced; e2e subset green. | 2 | mutation + e2e |
| **7 Data engineering** | dbt tests (unique, not_null, accepted_values, expression_is_true) per A7.4; DDL codegen round-trip; per-target dbt-PG vs dbt-CH dispatch test; macro unit tests via dbt `run-operation`. | 2 | integration (dbt) + golden (DDL) |
| **8 Agent schemas** | Golden fixtures for 11 agent schema round-trips; SKILL.md / command.md parser fuzz; content-type sniffer tests (xlsx/csv/pdf/docx/pptx/jsonl); eval runner against fixture dataset; gotcha auto-stale reactor test. | 2.5 | unit + golden + integration |
| **9 Hook & event spine** *(NEW Layer E)* | Heaviest test load: property tests for recursion guard, depth cap, hook ordering; `loom` model for `tdw-bus` under concurrency; idempotency suite for outbox + CDC double-publish; adversarial actor-leak test for raw `tokio::spawn`; schema drift gate; end-to-end actor traceability across 6 hops. | **4** | property + loom + integration + adversarial |
| **10 Snapshot / time travel** | 1000-write stress + flashback + undrop scenarios; vacuum correctness; snapshot tag survival across vacuum; snapshot-write overhead bench (Layer C R26). | 2 | integration + bench + property |
| **11 Streams + Live adapters** | Stream offset advancement under concurrent consumers; WebSocket subscribe + filter + DIFF + RLS; auto-resume after disconnect; perms-aware live (RLS in event path). | 1.5 | integration + e2e |
| **12 Graph + Spatial** | RELATE + traversal + recursive depth bounds + cycle protection; PostGIS distance + H3 + geohash round-trips. | 1.5 | integration + property |
| **13 Stages + table formats + pipes** | COPY INTO + auto schema evolution; PCRE2 pattern matching; Iceberg + Delta read via testcontainers MinIO + Iceberg REST catalog; ORC + AVRO format conversions; pipe polling + event-mode triggers. | 2 | integration + e2e |
| **14 UDFs + Auth + DEFINE + Masking** | Adversarial UDF sandbox suite (Python `open('/etc/passwd')`, JS prototype pollution, WASM fuel exhaustion, External UDF SSRF) — **all must fail safely**; JWT + JWKS rotation; SIGNIN/SIGNUP/AUTHENTICATE flow; record-user RLS; masking-bypass audit (R36); DEFINE schema two-run zero-diff. | **3.5** | adversarial + integration + property |
| **15 Hardening + E2E flows** *(NEW)* | Full pipeline e2e tests (see §8); mutation test gate enforced; performance baseline locked; flaky-test detection report; final docs pass. | **5** | e2e + mutation + perf |

**Total test-only added days**: ~37 days woven across phases + 4 (Phase 0.5) + 5 (Phase 15) = **~46 days** of test work.

**Important**: most of that is *interleaved* with dev work (you write tests as you go), not sequential. Net timeline impact: roughly **+10–12 days** of dedicated test work on top of phases that already include their own per-feature tests (Phase 0.5 + Phase 15 are net-new; per-phase test rows above are mostly already counted in the phase estimate).

---

## 8. Phase 15 — Hardening + E2E Flows — days ~101–105

End-of-project pass that catches what single-phase tests can't.

### Cross-phase E2E flows (each one a distinct test file)

15.1. **EOD ingest flow**: Polygon Fetcher (Phase 3) → ClickHouse write (Phase 2) → snapshot commit (Phase 10) → dbt bronze→silver→gold (Phase 7) → MCP query (Phase 5) → assert results match golden. ~30s, runs in `e2e-subset`.

15.2. **Research-note hybrid retrieval flow**: PDF upload (Phase 4) → S3 blob (Phase 4) → extract + embed via OpenAI provider (Phase 4) → Qdrant + Meilisearch index → HTTP hybrid search → MCP `tdw.documents.search_hybrid` → assert ranked top-10 stable. ~60s, e2e-full.

15.3. **Live trade flow with hooks**: Agent (Phase 8) emits `TradeIntent` event → spine routes via outbox (Phase 9) → masking pre-write hook (Phase 14) → PG insert → CDC ingress (Phase 9) → spine broadcast → ClickHouse mirror + Qdrant embed of trade memo + audit log + webhook notify (all async hooks) → live-WS subscriber receives event with diff (Phase 11) → audit table records `Actor::Agent` as root. ~45s, e2e-full.

15.4. **Time-travel rollback flow**: Insert 100 rows → snapshot → insert 100 more → tag snapshot A → 50 updates → `FLASHBACK TABLE ... TO SNAPSHOT A` → assert 200-row state restored → assert stream from snapshot-A onward captures all 50 updates as DELETEs+INSERTs. ~20s, e2e-subset.

15.5. **Graph traversal + spatial query flow** (Phase 12): RELATE 1000 vertices into a 3-layer graph + 100 spatial points → recursive traversal `.{1..3}` + PostGIS bounding box → assert combined query plan + correctness. ~30s, e2e-subset.

15.6. **Stages + Iceberg flow** (Phase 13): Upload Parquet to internal stage → COPY INTO with auto schema evolution → Iceberg read of same data via REST catalog → assert byte-identical rows. ~60s, e2e-full.

15.7. **UDF adversarial flow** (Phase 14): Run 8 hostile UDFs (Python file read, JS infinite loop, WASM out-of-memory, External SSRF, etc.) → assert each fails safely with the right error type → assert no process resource leakage (open fd, memory). ~120s, e2e-full.

15.8. **Auth + permissions flow** (Phase 14): SIGNUP → SIGNIN → bearer grant create → call masked endpoint as `viewer` → see `***` → rotate to `editor` role → see real values → revoke bearer grant → assert subsequent calls 401. ~30s, e2e-subset.

15.9. **Hook recursion flow** (Phase 9): Deliberately wire hook A → emits event X → hook B → emits event Y → hook A again → assert `MAXDEPTH = 8` triggers `DepthExceeded` at hop 9, structured error logged, no inconsistent state, no stuck transactions. ~20s, e2e-subset.

15.10. **Cross-actor traceability** (Phase 9 + 8): User HTTP call → agent invocation → sub-agent tool call → workflow step → DB write → assert audit trail captures full causation chain with correct actor at each hop. ~30s, e2e-full.

### Phase 15 work breakdown

15.A. Write 10 e2e flows (15.1–15.10). ~3 days.
15.B. Lock performance baseline in `docs/perf-history.md`; document the regression methodology. ~0.5 day.
15.C. Run full mutation suite workspace-wide; triage survivors; either fix tests or annotate `MUTANT-EQUIV/SKIP`. ~1 day.
15.D. Flaky-test detection: run integration + e2e ×10 in CI; quarantine tests that fail ≥ 2/10 to `tests/quarantine/` with linked issues. ~0.5 day.

**Exit criteria**: All 10 e2e flows pass in nightly CI for 7 consecutive days; mutation score ≥ tier floors; no quarantined tests; coverage workspace-aggregate ≥ 82%, all foundational crates ≥ 95%.

---

## 9. Test data strategy

| Dataset | Generator | Storage | Use |
|---------|-----------|---------|-----|
| OHLCV (deterministic synthetic) | `tdw-test-utils::fixtures::ohlcv()` with seeded RNG | In-memory | Most unit + integration tests |
| Instruments (real-world sample) | `seeds/instruments_top500.csv` (committed) | Repo | Phase 2, 3, 5 |
| Research notes (3 sample PDFs) | Manually curated, public-domain | `tests/fixtures/pdfs/` | Phase 4, 8 |
| Wiremock provider tapes | Recorded once via `--record` flag; replayed in CI | `tests/wiremock_tapes/` | Phase 3 |
| Eval datasets (fixture) | `tests/fixtures/eval/simple_math.parquet` | Repo | Phase 8 |
| SKILL.md / Command.md fixtures | Copied from user's `.claude/skills/` (sanitized) | `tests/fixtures/skills/` | Phase 8 |
| BOM schema golden files | Generated by `xtask events emit-schemas` | `schemas/events/` | Per-PR drift gate |
| DDL golden files | Generated by `xtask ddl-export` | `sql/ddl/golden/` | Per-PR drift gate |

**Reproducibility rule**: every test that uses random data passes a seed (`fixtures::ohlcv_with_seed(42)`); failures print the seed for replay. Tests never depend on system time without explicit injection (use `mock_instant` or `tokio::time::pause`).

---

## 10. Flaky-test policy

1. **Detection**: nightly job runs `integration` + `e2e` ×10. Tests failing ≥ 2/10 are reported.
2. **Quarantine**: failing tests move to `tests/quarantine/` immediately, with a linked tracking issue (`TODO(quarantine #NNN)`). Quarantine does NOT remove the test from coverage measurement; it just stops blocking merge.
3. **Triage SLA**: every quarantined test must have a triage note within 7 days (real bug vs flaky vs environment).
4. **Resolution**: real bugs → fix. Flaky → fix the race or use `#[retries(3)]`. Environment → split test or pin Docker image.
5. **Hard cap**: workspace has at most **5 quarantined tests at any time**; PR blocked if quarantine grows past 5.

---

## 11. CI/CD architecture

```
┌────────────────────────────────────────────────────────────────────────────┐
│  ON-PR JOBS (block merge)                                                  │
│  ┌─────────┐ ┌────────────┐ ┌──────────┐ ┌──────┐ ┌─────────────────┐    │
│  │  unit   │ │ integration│ │ coverage │ │ lint │ │  e2e-subset     │    │
│  │ Linux+  │ │   Linux    │ │  Linux   │ │      │ │   Linux         │    │
│  │ Win-MSVC│ │ ≤15 min    │ │  ≥82%    │ │      │ │   ≤20 min       │    │
│  │ ≤2 min  │ │            │ │   gate   │ │      │ │   ~5 flows      │    │
│  └─────────┘ └────────────┘ └──────────┘ └──────┘ └─────────────────┘    │
│                                                                            │
│  + windows-release (validates inventory + linkme on Win-MSVC release)      │
└────────────────────────────────────────────────────────────────────────────┘

┌────────────────────────────────────────────────────────────────────────────┐
│  NIGHTLY JOBS (report-only; failures opened as issues)                     │
│  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐ ┌────────────────┐   │
│  │   mutation   │ │  e2e-full    │ │    bench     │ │ flaky-detect   │   │
│  │  per-crate   │ │  all flows   │ │  perf-history│ │  10x integ+e2e │   │
│  │  ≤60 min ea  │ │  ≤60 min     │ │  regression  │ │                │   │
│  └──────────────┘ └──────────────┘ └──────────────┘ └────────────────┘   │
└────────────────────────────────────────────────────────────────────────────┘

┌────────────────────────────────────────────────────────────────────────────┐
│  PRE-RELEASE JOBS (manual trigger)                                         │
│  ┌──────────────┐ ┌──────────────┐ ┌──────────────────────────────────┐  │
│  │ full mutation│ │ 24h fuzz     │ │ docs build + link-check          │  │
│  │ workspace    │ │ on parsers   │ │                                  │  │
│  └──────────────┘ └──────────────┘ └──────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────────┘
```

### Concurrency rules

- `unit` and `lint` jobs run with **8 parallel** runners.
- `integration` and `e2e-subset` run with **2 parallel** (Docker memory cost).
- `coverage` is single-threaded, ~10 min.
- Cache: `Swatinem/rust-cache@v2` for cargo registry + target; cache key includes `Cargo.lock` hash + Rust toolchain version.

### Coverage report

- Codecov receives `lcov.info` from the `coverage` job.
- PR comment shows: workspace delta, per-crate delta, list of new uncovered lines.
- Block merge if any **foundational crate drops > 1pp** OR **workspace drops > 1pp**.

---

## 12. Acceptance Criteria

### Phase 0.5 — Test Foundation

A0.5.1. `cargo nextest run --workspace --no-default-features` completes < 2 min on Linux, < 4 min on Windows-MSVC, with zero unit-test failures.
A0.5.2. `docker compose --profile minimal up -d && cargo nextest run --workspace --features integration` completes < 15 min on Linux with zero integration failures (initially against a no-op test).
A0.5.3. `cargo llvm-cov nextest --workspace --lcov --output-path lcov.info` produces a valid lcov file, and Codecov ingestion succeeds.
A0.5.4. `xtask bench` runs Criterion benches in a stable directory, writes structured JSON to `docs/perf-history.json`.
A0.5.5. `Justfile` exposes: `test-unit`, `test-integration`, `test-property`, `test-e2e`, `coverage`, `coverage-html`, `bench`, `bench-compare`, `mutation <crate>`, `lint`, `fmt`. Each runs to completion on a fresh checkout.
A0.5.6. CI matrix is green on the empty workspace (no real code yet).
A0.5.7. `docs/testing.md` documents the six tiers, naming conventions, and adversarial-test policy.
A0.5.8. `tdw-test-utils::containers::*` factory functions exist for postgres, clickhouse, qdrant, meilisearch, minio, redis; `cargo test -p tdw-test-utils --features integration` runs a no-op test against each container.

### Phase 15 — Hardening + E2E Flows

A15.1. All 10 cross-phase e2e flows (§8) pass for **7 consecutive nightly runs** before declaring v0.1 ready.
A15.2. Mutation score ≥ tier floors (§5) for every crate; survivors annotated.
A15.3. Workspace coverage **≥ 82% line, ≥ 78% branch**; foundational crates **≥ 95% line**.
A15.4. Zero quarantined tests at v0.1 release.
A15.5. `docs/perf-history.md` shows ≤ 20% regression on every named workload vs the rolling-7-day baseline.
A15.6. CI `windows-release` job (Layer C A10 + Layer E R57) green for 7 consecutive nightly runs.
A15.7. `xtask events schema-check` (Layer E V59) and `xtask schema-sync` (Layer A V14) both green in CI.
A15.8. Adversarial test suite (UDF sandbox, masking bypass, recursive hook, raw `tokio::spawn`) all green; no regressions in security gates.
A15.9. Flaky-test report shows ≤ 0 tests failing ≥ 2/10 in nightly ×10 runs.
A15.10. Documentation: every `docs/testing.md` claim verified; per-crate `README.md` describes how to run that crate's tests.

### Per-phase coverage gates (added to every phase exit criteria)

For every Phase N exit: the phase's new crates (and modifications to existing crates) must satisfy:

- New code has **≥ 80% line coverage** (≥ 95% for foundational crates per §5).
- All new public functions have at least one unit or integration test.
- All new traits have a property test for their laws (where laws can be stated).
- All new database tables have at least one round-trip integration test.
- All new event types have a golden serde fixture.
- All new error variants have at least one test that produces them.

A phase's "Exit criteria" list (see parent plans) **MUST** be extended with: `Test gates from Layer F §5 satisfied.` This is a single line that brings all of §5 in scope.

---

## 13. Risks & Mitigations

| #    | Risk | Likelihood | Impact | Mitigation |
|------|------|-----------|--------|------------|
| R59  | Coverage gate forces shallow "coverage-padding" tests | High | Medium | Mutation testing (§5) catches tests that increase coverage without catching bugs; mutation-score floor is a second gate. |
| R60  | `testcontainers` on Windows-MSVC is 3-5× slower (parent R11) makes integration suite slow | High | Medium | Skip the `integration` and `e2e-subset` jobs on Windows-MSVC; only `unit` + `lint` + `windows-release` run on Windows. Linux runs the heavy tiers. Documented in `docs/testing.md`. |
| R61  | Flaky CI erodes trust in the gate | High | High | §10 policy: quarantine within 7 days, triage SLA, hard cap of 5 quarantined tests. |
| R62  | E2E tests pull in cloud APIs (OpenAI / Google embeddings) and rack up cost | Medium | Medium | All e2e tests against real APIs are tagged `#[e2e(billed)]` and run only on manual trigger; default e2e uses `tdw-embed-local` (Phase 4); per-test cost cap in `docs/testing.md`. |
| R63  | Mutation testing is too slow to run on every PR | High (60+ min/crate) | Low | Nightly per-crate; quarterly full workspace; PR shows mutation diff only for changed crates. |
| R64  | Coverage measurement disagrees between local and CI (LLVM versions, debug vs release) | Medium | Low | `Justfile coverage` uses the same `cargo llvm-cov` flags as CI; lockfile pinned via `rust-toolchain.toml`. |
| R65  | Property-test shrink budget too aggressive on PR runs (slow) → too small on nightly (misses bugs) | Medium | Medium | PR uses `PROPTEST_CASES=64`, nightly uses `PROPTEST_CASES=1024`; counterexamples persisted in `proptest-regressions/` and committed. |
| R66  | `loom` tests for `tdw-bus` explode in state space | High | Medium | Bound `loom` model depth; run only on PRs touching `tdw-bus`; nightly runs full budget. |
| R67  | Test data drifts from production schema (golden files become stale) | High | Medium | `xtask events schema-check` + `xtask schema-sync` + `xtask ddl-export` are CI gates that reject drift. |
| R68  | Adversarial UDF tests are interpreted as malicious by host OS (AV, EDR) | Medium | Low | Adversarial tests run in `--features adversarial` and only inside `testcontainers` (sandboxed); documented warning to disable host AV on test directory if needed. |

---

## 14. Verification Steps

V62. `just test-unit` completes < 2 min on a clean checkout, Linux. (A0.5.1)
V63. `just test-integration` completes < 15 min on a clean checkout, Linux, with Docker Desktop running. (A0.5.2)
V64. `just coverage` produces `target/coverage/html/index.html`; opening it shows per-crate breakdown. (A0.5.3)
V65. PR with intentional 5pp coverage drop is blocked by CI. (Gate)
V66. PR with weak test (uses `assert!(true)`) survives in coverage but is killed by `cargo mutants` nightly. (R59, A15.2)
V67. PR that introduces a new event type without a golden fixture fails CI on `xtask events schema-check`. (Phase 0.5 + R67)
V68. PR that breaks an existing serde round-trip is caught by `insta` snapshot review and blocked. (A8.11)
V69. PR introducing a hook without an actor context is caught by Clippy lint (Layer E R51) and blocked. (V58)
V70. PR with a > 20% perf regression on any named workload is reported in PR comment and blocked. (A15.5)
V71. Quarantine count > 5 blocks merge. (§10, A15.4)
V72. Windows-MSVC release-profile job catches an `inventory`-registration regression on a deliberate test. (Layer C R6, Layer E R57)
V73. Adversarial test suite green: all 8 hostile UDFs in Phase 15.7 fail safely; no host process leaks. (A15.8)
V74. End-to-end actor traceability (E2E 15.10) shows correct causation chain for a 6-hop event. (Layer E V61, A15.1)

---

## 15. ADR

- **Decision**: Adopt a six-tier test pyramid (unit · integration · property · e2e · mutation · bench) with crate-tier coverage floors (foundational ≥ 95%, storage ≥ 85%, providers ≥ 80%, workspace aggregate ≥ 82%), mutation-score floors (foundational ≥ 85%, workspace ≥ 75%), shared `tdw-test-utils` infrastructure delivered in a new Phase 0.5, and a final Phase 15 for cross-phase e2e flows + hardening. CI gates every PR on unit + integration + coverage + lint + e2e-subset; nightly runs mutation + e2e-full + bench + flaky detection.

- **Drivers**:
  1. Single-person ownership amplifies the need for automated quality gates.
  2. ~47-crate workspace cannot be manually smoke-tested.
  3. Foundational crates (spine, outbox, masking, auth) carry every other crate's correctness.

- **Alternatives considered**:
  - **B — Unit + integration only**: rejected — spine + UDF sandbox demand property + adversarial.
  - **C — Manual + smoke**: rejected — single-person ownership amplifies risk.

- **Why chosen**: explicit tier costs/values; shared infra avoids duplication; mutation testing prevents coverage-padding; tier-specific floors prevent the "all crates at 80%" anti-pattern that under-tests critical code.

- **Consequences**:
  - +4 days (Phase 0.5) + 5 days (Phase 15) = **+9 days** of dedicated test phases.
  - +10–12 days of per-phase test work woven in (most already counted in phase estimates).
  - Workspace total timeline: ~100 days → **~110 days serial / ~75 days parallelized**.
  - Coverage + mutation gates may surface real bugs that block PRs longer than expected; this is the intended outcome.
  - Adversarial test suite + sandbox testing make Phase 14 more expensive but materially safer.

- **Follow-ups**:
  - ADR-0026 — coverage tier floors (this plan, §5)
  - ADR-0027 — mutation-score gating policy
  - ADR-0028 — flaky-test quarantine + triage SLA
  - ADR-0029 — e2e cost containment (no-billed-call default)
  - O24 — should mutation testing run on every PR for changed crates only? (Default: no, nightly only; PR shows nightly delta.)
  - O25 — when to introduce `loom` more broadly (only `tdw-bus` for v0.1 — expand at v0.2?)
  - O26 — fuzz testing target list — which parsers / wire formats?

---

## 16. Combined timeline (updated, with Phase 0.5 + Phase 15)

| Phase | Description | Days | Cumulative |
|-------|-------------|------|------------|
| 0.0 | Discovery (BOM re-derive, ADRs, license) | 0–2 | 2 |
| 0.1 | Workspace skeleton + CI matrix | 1 | 3 |
| **0.5 (NEW)** | **Test Foundation** | **4** | **7** |
| 1 | Core abstractions | 2–5 | 12 |
| 2 | Storage engines | 6–10 | 17 |
| 3 | First providers | 11–13 | 20 |
| 4 | Hybrid retrieval + 3 embedders | 14–20 | 27 |
| 5 | Four consumer shells | 21–26 | 33 |
| 6 | Hardening & docs | 27–32 | 39 |
| 7 | Data engineering (dbt, SQL, ETL/ELT, DDL codegen) | 33–42 | 49 |
| 8 | Agent schemas (12 types, MCP tools, eval runner) | 43–49 | 56 |
| 9 (Layer E) | Hook & event spine | 50–59 | 66 |
| 10 (was 9) | Snapshots / time travel / tags | 60–68 | 74 |
| 11 (was 10) | Streams + live queries (adapters) | 69–72 | 77 |
| 12 (was 11) | Graph + spatial | 73–80 | 84 |
| 13 (was 12) | Stages + table formats + pipes | 81–90 | 94 |
| 14 (was 13) | UDFs + auth + DEFINE + masking | 91–100 | 104 |
| **15 (NEW)** | **Hardening + E2E flows** | **101–108** | **111** |

Total **~111 days serial / ~75 days with parallelization** (Phase 7 overlaps Phase 8; Phases 12+13 overlap 14; per-phase test work parallelizes with feature work).

---

## 17. What changes in every other plan

This Layer F plan does NOT touch the implementation phases' core scope. It binds them via a single rule:

> **Every phase exit criteria gain one line:**
> "Test gates from Layer F §5 satisfied — coverage ≥ tier floor, mutation ≥ tier floor, all new public API has tests, all new event types have golden fixtures."

Concretely, in the existing plan files:
- `2026-05-21-rust-trading-data-warehouse.md` Phase 1–6 exit criteria: add the line.
- `2026-05-21-data-engineering-and-agent-schemas.md` Phase 7–8 exit criteria: add the line.
- `2026-05-21-hook-event-spine.md` Phase 9 exit criteria: add the line. Spine acceptance criteria already cover property tests, recursion, idempotency — Layer F just adds the coverage gate.
- `2026-05-21-databend-surrealdb-feature-parity.md` Phase 10–14 exit criteria: add the line.

The line acts as a forward reference; the detailed rules live here.

---

## 18. Open Questions

- **O24** — Mutation testing on PRs (only for changed crates) vs nightly-only?
- **O25** — `loom` coverage beyond `tdw-bus` for v0.1?
- **O26** — Fuzz testing target list (parsers / wire formats / SKILL.md / DEFINE schema)?
- **O27** — Codecov vs Coveralls vs self-hosted coverage UI?
- **O28** — Should `windows-release` job run on every PR, or nightly only? (Default: every PR — it catches `inventory`/`linkme` issues early.)
- **O29** — Performance baseline: per-machine (user's Windows 11) or normalized to a fixed Linux CI runner? (Default: fixed CI runner; user-machine numbers tracked separately for context.)

---

## 19. Changelog

**2026-05-21 — Layer F: Test Strategy**
- Six-tier test pyramid: unit · integration · property · e2e · mutation · bench.
- Crate-tier coverage floors (foundational ≥ 95%, workspace ≥ 82%) + mutation-score floors.
- New Phase 0.5 (Test Foundation, 4 days) for shared `tdw-test-utils`, CI matrix, coverage tooling, Justfile.
- New Phase 15 (Hardening + E2E flows, ~5 days) with 10 cross-phase e2e flows.
- ~10 days of per-phase test work woven in (mostly already counted).
- CI architecture: on-PR (unit + integration + coverage + lint + e2e-subset + windows-release) · nightly (mutation + e2e-full + bench + flaky-detect) · pre-release (full mutation + 24h fuzz + docs).
- 13 acceptance criteria (A0.5.1–A0.5.8 + A15.1–A15.10) + per-phase gate line.
- 10 risks (R59–R68), 13 verification steps (V62–V74), 4 follow-up ADRs (0026–0029) + 6 open questions (O24–O29).
- Total project timeline: ~100 days → **~111 days serial / ~75 parallelized**.
