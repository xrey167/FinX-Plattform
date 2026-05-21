## Summary

## Verification

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cargo run -p xtask -- clean-room-audit`

## Clean-Room Checklist

- [ ] No `finx-*` dependencies.
- [ ] No copied FinX-XR code or trait signatures.
- [ ] No `tdw-provider-openbb`.
