# FinX-Finance — `connect-rust` + `buffa` Integration Evaluation

**Project:** FinX-Finance (`C:\Users\ReyDa\FinX-Finance\`)
**Date:** 2026-05-21
**Mode:** `/omc-plan --direct` (focused addendum)
**Status:** Decision recorded — both repos evaluated; no v0.1 plan changes required
**Parent plans:**
- [`2026-05-21-rust-trading-data-warehouse.md`](./2026-05-21-rust-trading-data-warehouse.md) — core (Phases 0–6)
- [`2026-05-21-data-engineering-and-agent-schemas.md`](./2026-05-21-data-engineering-and-agent-schemas.md) — Layer A+B (Phases 7–8)
- [`2026-05-21-databend-surrealdb-feature-parity.md`](./2026-05-21-databend-surrealdb-feature-parity.md) — Layer C (Phases 9–13)

---

## 1. Repo facts (verified 2026-05-21)

| | `anthropics/connect-rust` | `anthropics/buffa` |
|---|---|---|
| **What** | Rust impl of [ConnectRPC](https://connectrpc.com) protocol — one server speaks Connect + gRPC + gRPC-Web simultaneously over HTTP/1.1 or HTTP/2 (binary or JSON protobuf) | Pure-Rust protobuf runtime with first-class **editions** support; replaces `Option<Box<T>>` ergonomics, adds zero-copy `View<'a>` types |
| **License** | Apache-2.0 | Apache-2.0 |
| **Stars / commits / releases** | 378 / 88 / **11 releases** (v0.6.0, 2026-05-20) | 733 / 89 / **8 releases** (v0.6.0, 2026-05-15) |
| **Maturity** | Pre-1.0; passes full upstream Connect conformance suite (3,600 server + 6,872 client tests); CI green | Pre-1.0; passes full protobuf conformance suite; CI green |
| **MSRV** | 1.88 | 1.85 |
| **Anthropic-internal assumptions?** | **None.** Generic OSS port; no internal hostnames, no Anthropic auth, no proprietary deps | **None.** Public technical spec; standalone runtime |
| **Direct dep on the other** | **Yes** — pulls `buffa` as its protobuf runtime (unavoidable transitive) | No (standalone) |
| **Tonic / prost in tree?** | **No** — parallel stack, not a wrapper | No |

Both are real, both work, both are functionally first-class. Neither is "Anthropic-internal" beyond living under the `anthropics` GitHub org.

---

## 2. Fit assessment against FinX-Finance

| Component currently in plan | connect-rust fit | buffa fit |
|-----------------------------|------------------|-----------|
| `tdw-service` axum HTTP server | **Drop-in via `axum` feature** — `connect.into_axum_service()` becomes an axum `Router` fallback on a `/connect.*` route, co-exists with existing handlers | N/A |
| `tdw-service` tonic gRPC | Functional **replacement** (and then some — same wire + browser-callable). 1.35–1.95× faster unary in their benchmarks. But generated code is not interchangeable with tonic | N/A |
| `tdw-mcp` (MCP server, stdio + HTTP/SSE) | **No fit.** MCP has its own transport spec; Connect is irrelevant unless we expose parallel RPCs for non-MCP clients | N/A |
| `tdw-domain` Rust structs + serde + schemars + JsonSchema + Validate | N/A | **No fit.** We have zero `.proto` files. Adopting buffa standalone = building a parallel protobuf schema layer alongside serde/JsonSchema for no benefit |
| `sqlx` + `dbt` for SQL transforms | N/A | N/A |
| Code generation (`tdw-sql-codegen`) | N/A | N/A |
| Cross-language SDKs | **Real win** — clients in TypeScript / Go / Swift / Kotlin via Connect's existing codegen suite, all calling the same server | Indirect — only via connect-rust |

**Score / verdict:**
- **connect-rust: 3/5 — OPTIONAL.** Technically superior to tonic for our shape, but migrating a working stack for marginal gains in v0.1 isn't worth it. Park as a candidate for a future browser-facing query API.
- **buffa: 1/5 — SKIP (standalone) or TRANSITIVE-ONLY (if connect-rust adopted later).** No protobuf in our stack; no editions problem to solve.

---

## 3. When to revisit (decision criteria)

connect-rust becomes the right call **only if** one of these is true:

1. **Browser dashboard ships.** We build a TypeScript/Svelte/React UI that needs to call FinX-Finance directly (not via PG-wire BI tools). Connect + gRPC-Web on the same axum endpoint removes the need for a separate `tonic-web` proxy, and the Connect TypeScript codegen gives the dashboard a typed client for free.
2. **Public RPC API for external consumers.** If FinX-Finance ever serves external clients (other researchers, a Discord bot, a Slack integration), Connect's HTTP/JSON mode means callers without a gRPC client (curl, fetch, requests) just work.
3. **Cross-language agent SDKs needed.** If the agent layer (Phase 8) grows to support non-Rust agents (Python LangChain consumers, TypeScript agents in browsers), Connect's polyglot codegen beats hand-rolled HTTP wrappers.
4. **tonic 1.0 stalls or regresses.** If tonic's maintenance slows or breaks our use cases, Connect is the cleanest alternative.

If none of (1)–(4) is true, **stay on tonic** for the gRPC surface and skip connect-rust entirely. buffa never enters except as a transitive dep when (and if) connect-rust does.

---

## 4. Concrete plan changes

### 4.1 Phase 5 — Consumer Shells (parent plan, lines 357–365)

**Change**: add a *documented hook* in `tdw-service` for an optional `tdw-service-connect` crate, without committing to it.

Updated workspace layout (additive, all `[future]` paths):

```diff
crates/
  tdw-service/                            # axum HTTP + tonic gRPC (unchanged)
+ tdw-service-connect/                    # [future, behind --feature connect]
+   src/lib.rs                            # exposes the same CommandRunner via ConnectRPC
+   build.rs                              # uses connectrpc-build if enabled
+ proto/                                  # [future, only if connect adopted]
+   finx_finance/v1/                      # .proto definitions mirroring tdw-domain
```

**Implementation rule**: until criterion #1, #2, or #3 in §3 fires, the `tdw-service-connect/` directory does not exist. The plan **does not** add Phase work for it.

### 4.2 ADR-0020 — RPC transport boundary

Add to `docs/adr/`:

```
ADR-0020: RPC transport boundary

Decision: axum HTTP + tonic gRPC are the v0.1 RPC layer. ConnectRPC
(via anthropics/connect-rust) is evaluated and parked.

Drivers:
  1. We have a working axum+tonic stack; migration cost is real.
  2. v0.6.0 is pre-1.0 for connect-rust and buffa — API may shift.
  3. No current consumer (BI tools, dashboards, MCP) needs Connect.

Alternatives considered:
  - Adopt connect-rust now: rejected — premature for a personal warehouse.
  - Skip Connect permanently: rejected — leaves no plan for browser RPC if it ever ships.
  - Hybrid (tonic for gRPC, connect for HTTP/Connect): viable but doubles maintenance.

Why parked, not skipped:
  Connect-rust solves one problem we don't have *yet* (browser-callable RPC
  on the same axum endpoint without a grpc-web proxy). The trigger conditions
  to revisit are documented in plan §3.

Consequences:
  - tdw-service stays on tonic for v0.1; tdw-service-connect/ is reserved as
    a future opt-in crate.
  - No protobuf source-of-truth alongside tdw-domain at v0.1.
  - Decision review on the next major roadmap milestone (post-Phase 8).

Follow-ups:
  - Re-evaluate when (a) Connect / buffa hit 1.0, OR (b) any §3 trigger fires.
```

### 4.3 Memory entry

Save a memory note (already in scope per /memory hooks) so future sessions don't waste cycles re-evaluating:

```
project_finx_finance — connect-rust + buffa evaluated 2026-05-21.
Both Apache-2.0, public, pre-1.0 (v0.6.0). Verdict: OPTIONAL/SKIP for v0.1;
revisit only on browser-dashboard or external RPC API trigger. Plan stays
on axum + tonic. ADR-0020 records.
```

---

## 5. What does NOT change

- **Phase 5 timeline** stays at 21–26 days. No new crate is built.
- **All existing acceptance criteria** A5/A6/A7/A9 (HTTP, worker, MCP, streaming runner) remain on axum/tonic.
- **tdw-domain** stays serde + schemars + JsonSchema + Validate. No `.proto` files in v0.1.
- **No new risks** are introduced. The decision to *not* adopt is the safe path.

---

## 6. Risks (small, of *adopting* later)

| # | Risk if/when we adopt connect-rust | Mitigation |
|---|------------------------------------|------------|
| R40 | Pre-1.0 API shift between v0.6 and v1.0 forces rework | Pin to a single minor; upgrade in one PR; isolate behind `tdw-service-connect` so it doesn't bleed into `tdw-runtime` |
| R41 | buffa enters as a transitive dep, dragging another protobuf runtime alongside prost (if anything else pulls prost) | Avoid simultaneous `prost` + `buffa` in the dep tree; pick one runtime if Connect is adopted |
| R42 | Connect's TypeScript codegen evolves separately from `connect-rust`; client-server protocol drift | Use the same Connect spec version on both sides; CI conformance test against `connectrpc/conformance` |
| R43 | If both Connect and tonic services coexist, doubled maintenance + confused client onboarding | Decision rule: pick one wire per *service*, not per *route*. Document in CONTRIBUTING. |

---

## 7. Verification (none new for v0.1)

No new tests required. The decision is documented in:
- ADR-0020 (`docs/adr/0020-rpc-transport-boundary.md`) — to be authored in Phase 6.
- This evaluation file — read-only reference for the decision.

If/when connect-rust is later adopted:
- A new V48 will fire `cargo test -p tdw-service-connect --features integration` against the upstream `connectrpc/conformance` suite.
- A new V49 will verify TypeScript client (generated from same `.proto`) round-trips a sample equity_historical query.

---

## 8. Changelog

**2026-05-21 — initial evaluation**
- Researched `anthropics/connect-rust` (Connect protocol Rust impl, Apache-2.0, v0.6.0, conformance-passing) and `anthropics/buffa` (protobuf editions runtime, Apache-2.0, v0.6.0, conformance-passing).
- Verdict: connect-rust OPTIONAL (3/5), buffa SKIP (1/5) for v0.1.
- Added §3 trigger conditions for future adoption.
- Added ADR-0020 stub (full ADR authored in Phase 6).
- Added R40–R43 to risk register *contingent on adoption*.
- No phase / acceptance-criteria changes to v0.1.
- Reserved `crates/tdw-service-connect/` and `proto/` paths in workspace layout as `[future, opt-in]`.
