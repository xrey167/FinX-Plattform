# FinX-Plattform

Private Rust workspace for the FinX-Finance trading data warehouse plan set.

This repo is intentionally clean-room relative to FinX-XR:

- crate names use the `tdw-*` prefix;
- no `finx-*` dependencies are allowed;
- no `tdw-provider-openbb` crate exists;
- GitHub setup is local until a remote owner/name/visibility is explicitly chosen.

## Bootstrap Commands

```powershell
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo run -p xtask -- clean-room-audit
```

The source plans are copied into `.plans/` for traceability.

Docker profile and WSL2 volume guidance lives in `docs/docker.md`.
