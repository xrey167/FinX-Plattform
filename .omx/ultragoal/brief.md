Create a new durable ultragoal for FinX-Plattform to audit and harden every workspace crate under crates/* crate by crate for production-grade readiness. Do not create one fragile goal per crate; create grouped tranche goals by architecture layer, but every crate must receive its own readiness assessment artifact. For each crate, check whether it is production-grade, battle-tested, covered by meaningful unit/integration/golden/property tests where applicable, documented, fully wired into dependent crates and public surfaces, correctly represented in workspace dependencies, correctly represented in feature flags where features are needed, and fully functional rather than scaffold-only. Preserve AGENTS.md clean-room constraints: no finx-* dependencies, no copied FinX-XR code, no tdw-provider-openbb. Required per-crate rubric: manifest correctness, dependency direction, feature flags, public API and error handling, runtime behavior, tests and coverage evidence, docs and examples, wiring through service-api/cli/mcp/xtask if applicable, dead code/scaffold/fallback scan, security and reliability risks, and explicit production-readiness verdict. Produce docs/quality/crate-readiness/<crate>.md or equivalent indexed artifacts plus an aggregate matrix. Final story must run targeted verification, ai-slop-cleaner on changed files, full workspace verification, and code-review with APPROVE and architectStatus CLEAR before completing the Codex goal.

## Production Functionality Push (G009-G016)

After the bootstrap readiness audit landed (G001-G008, completed and merged on main as commit 50e094a), this directory pursues a second ultragoal: take every crate from "Ready with follow-ups" bootstrap-contract state to actual production-functional. Each crate's "Follow-up boundary" notes in its readiness worksheet become the implementation target for the matching tranche below.

Stories:
- G009 End-to-end functional smoke: prove the bootstrap composition functions as a working system; document the smoke recipe.
- G010 Production storage transports: real ClickHouse, Postgres, S3, Qdrant, Meilisearch network clients with dockerized integration tests.
- G011 Production provider transports: real HTTP execution for Alpaca, Binance, FRED, Polygon, Yahoo, HuggingFace providers with replay-cassette CI.
- G012 Production LLM and embedding transports: real Anthropic, OpenAI-compatible, OpenAI-embed, Google-embed adapter execution.
- G013 Durable persistence: Postgres-backed outbox, session store, persistent bus retention, durable snapshot and rollout.
- G014 Release packaging: per-binary Dockerfiles, docker-compose full-stack, GitHub release workflow, semver policy, ADR 0013.
- G015 Policy enforcement binding: hooks, auth, sandbox, mask wired into actual request-path enforcement with permission-deny integration tests.
- G016 Aggregate production-functional gate: final verification, summary update, APPROVE/CLEAR code review.

Constraints preserved: clean-room (no finx-*, no copied FinX-XR, no tdw-provider-openbb); per-crate readiness worksheets updated as production evidence lands; quality gates green at each story completion; live-network and live-credential tests gated behind feature flags so the default workspace test stays offline.
