# FinX-Plattform Agent Rules

- Treat this directory as the Git/workspace root.
- Preserve the clean-room boundary: no `finx-*` crates or dependencies, no copied
  FinX-XR code, and no `tdw-provider-openbb`.
- Keep changes scoped and verified with fresh command output.
- Use `cargo fmt --all -- --check`, `cargo check --workspace`, `cargo test --workspace`,
  and `cargo run -p xtask -- clean-room-audit` for bootstrap-level verification.
- Do not create or push a GitHub remote without explicit user approval for owner,
  repository name, and visibility.
