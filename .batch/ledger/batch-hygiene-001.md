---
batch: batch-hygiene-001
items: hygiene:advisories
outcome: done
---

# batch-hygiene-001 — jsonwebtoken security bump (dependabot alert #4)

First hygiene batch, triggered directly by the repeated dependabot alert
rather than a scan item (the next discover folds this in).

## Scope

- `jsonwebtoken` 9.3.1 → 10.4.0 (alert #4: type confusion → potential
  authorization bypass in the JWT verification path; patched ≥10.3.0).
  Declared in root `Cargo.toml` `[workspace.dependencies]`; consumed by
  `tdw-auth-oidc`. No source changes needed — the used API surface is
  unchanged across the major bump.
- Backend choice: `rust_crypto` feature (pure-Rust rsa/p256/p384/hmac/sha2)
  instead of jsonwebtoken 10's default `aws-lc-rs` — avoids C/CMake/NASM
  build deps that would break the Windows CI matrix.

## ⚠ Security trade-off (documented, vetoable)

The `rust_crypto` backend pulls `rsa 0.9.10`, which carries
**RUSTSEC-2023-0071** (Marvin timing sidechannel, no fixed version).
Marvin attacks RSA PRIVATE-key operations; tdw-auth-oidc performs only
PUBLIC-key signature verification of IdP JWTs, so the vulnerable path is
never exercised. Accepted via a justified `deny.toml` ignore in preference
to (a) keeping the real auth-bypass vuln or (b) breaking CI with aws-lc-sys.
Revisit when https://github.com/RustCrypto/RSA/issues/626 resolves.

## Gate evidence

| Gate | Command | Result |
| --- | --- | --- |
| deny | `cargo deny check` | advisories ok, bans ok, licenses ok, sources ok |
| tests (crate) | `cargo test -p tdw-auth-oidc` | 13 passed, 0 failed |
| tests (workspace) | `cargo test --workspace` | pass (0 failed) |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| fmt | `cargo fmt --all -- --check` | pass |
| clean-room | `cargo run -p xtask -- clean-room-audit` | pass |

## PR

(link added on creation)
