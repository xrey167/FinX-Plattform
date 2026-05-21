# ADR-0001: Relationship To FinX-XR

## Decision

FinX-Plattform is a clean-room private Rust project. FinX-XR may be read only for
high-level lessons when a plan explicitly asks for it, but code, trait signatures,
tests, and module contents must not be copied.

## Consequences

- Crates use the `tdw-*` prefix.
- `finx-*` dependencies are rejected by review and `xtask clean-room-audit`.
- `tdw-provider-openbb` is not created.
