# ADR-0002: Workspace Bootstrap

## Decision

Create compile-ready stubs for the plan-backed workspace up front, then implement
features phase by phase.

## Drivers

- Parallel work needs stable package paths.
- CI should catch dependency and clean-room drift early.
- Empty folders are not enough; the workspace must compile.

## Consequences

- Many crates start as intentionally small stubs.
- Public contracts begin in `tdw-core`, `tdw-domain`, and `tdw-runtime`.
