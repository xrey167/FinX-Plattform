# FinX-Plattform

Rust workspace for the FinX-Finance trading data warehouse plan set. Hosted at
[github.com/xrey167/FinX-Plattform](https://github.com/xrey167/FinX-Plattform)
(public — visibility chosen so GitHub Actions and branch protection are usable
on the Free tier; the workspace was originally drafted as a private project).

This repo is intentionally clean-room relative to FinX-XR:

- crate names use the `tdw-*` prefix;
- no `finx-*` dependencies are allowed;
- no `tdw-provider-openbb` crate exists;
- the remote owner/name/visibility is explicit and tracked in `AGENTS.md`.

## Bootstrap Commands

```powershell
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo run -p xtask -- clean-room-audit
```

The source plans are copied into `.plans/` for traceability.

Docker profile and WSL2 volume guidance lives in `docs/docker.md`.
