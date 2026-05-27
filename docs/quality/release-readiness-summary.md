# Release Readiness Summary

Scope: G001-G016 aggregate readiness for the FinX-Plattform clean-room TDW workspace. G001-G008 remain the bootstrap crate-readiness baseline; G009-G016 are the production-functional push.

Verdict: APPROVE / CLEAR for the production-functional release gate.

Current release evidence:
- Main commit: `af6f5e4e243e6fc1fdd9c574f47f7f3564a494fe`.
- Main CI: PASS, run `26543584954`, completed on 2026-05-27.
- Main CodeQL: PASS, run `26543584951`, completed on 2026-05-27.
- Release tag: `v0.1.1`.
- Release URL: `https://github.com/xrey167/FinX-Plattform/releases/tag/v0.1.1`.
- Release workflow: PASS, run `26543606204`.
- Release assets: 24 uploaded assets, covering `tdw-service`, `tdw-cli`, `tdw-mcp`, and `tdw-worker` for Linux x86_64, macOS arm64, and Windows x86_64, plus SHA-256 checksum files.
- Build provenance: the release workflow ran `actions/attest-build-provenance@v3` for archives and checksum files in each build job.
- Container evidence: main CI includes GHCR image build, Trivy scan, push-on-main behavior, and dockerized service/worker smoke jobs.

Production-functional coverage:
- G009 End-to-End Functional Smoke: baseline deterministic service/API smoke is documented in `docs/quality/end-to-end-smoke.md` and transcripted in `docs/quality/end-to-end-smoke-transcript.md`.
- G010 Production Storage Transports: Postgres, S3, ClickHouse, Qdrant, and Meilisearch real adapters landed behind feature/env gates with CI-backed integration coverage.
- G011 Production Provider Transports: Yahoo, FRED, Polygon, Alpaca, Binance, and HuggingFace real HTTP transports landed with cassette tests and opt-in live tests.
- G012 Production LLM and Embedding Transports: Anthropic and OpenAI-compatible chat clients, SSE streaming, OpenAI embeddings, and Google embeddings landed behind feature/env gates.
- G013 Durable Persistence: Postgres-backed outbox, bus, session, and snapshot paths plus locked/synced filesystem rollout persistence landed with gated durable-store coverage.
- G014 Release Packaging: per-binary Dockerfiles, full-stack compose profiles, CI image scan/push, release workflow, ADR 0013, and SemVer/pre-1.0 policy landed.
- G015 Policy Enforcement Binding: service request paths now enforce OIDC claims, deny-by-default auth roles, policy-gated hook handler execution, sandbox UDF capabilities, and response masking.
- G016 Aggregate Gate: final release workflow, release assets, CI/CodeQL evidence, final quality gate, final code review, and `.omx` completion evidence are captured in this branch.

Resolved release issue:
- The first tag attempt, `v0.1.0`, built all matrix artifacts but failed in the publish job because `gh release create` ran from an artifact-only workspace and could not infer the repository from `.git`.
- PR #45 fixed the workflow by setting `GH_REPO` from `github.repository`.
- `v0.1.1` is the first successfully published production-functional release. The failed `v0.1.0` tag remains historical evidence and was not rewritten.

Bootstrap baseline:
- G001-G008 already reached APPROVE / CLEAR for crate-readiness hardening.
- Every workspace crate has a worksheet under `docs/quality/crate-readiness/`.
- The aggregate matrix has no pending tranche-audit verdicts.
- Clean-room audit evidence remains valid: no `finx-*` crate/dependency, copied FinX-XR code, or `tdw-provider-openbb` dependency was introduced.

Final verification set:
- `cargo +stable fmt --all -- --check`
- `cargo +stable check --workspace`
- `cargo +stable clippy --workspace --all-targets -- -D warnings`
- `cargo +stable test --workspace`
- `cargo +stable run -p xtask -- clean-room-audit`
- `git diff --check`
- `.omx/ultragoal/goals.json` JSON parse and `.omx/ultragoal/ledger.jsonl` JSONL parse
- G016 AI slop cleanup pass over changed evidence files and the release workflow fix

Residual follow-ups:
- GitHub Actions currently reports Node.js 20 deprecation annotations for `actions/upload-artifact@v4` and `actions/download-artifact@v4`; these are warnings, not gate blockers.
- `windows-latest` runner redirection notices are informational.
- Local Docker is not installed on this workstation, so dockerized compose execution evidence comes from GitHub CI rather than a local run.
