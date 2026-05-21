# BOM Schema Provenance

The v0.1 business object model is re-derived for this clean-room project. It is
not copied from FinX-XR or OpenBB. The canonical implementation lives in
`crates/tdw-domain`; these markdown specs document the same field contracts for
review and drift checks.

Source classes used for synthesis:

- Exchange and broker-neutral market data vocabulary: OHLCV bars, order events,
  position snapshots, trading sessions, and fees.
- Public finance reporting vocabulary: fundamentals by symbol, fiscal period,
  metric, value, currency, and report timestamp.
- Operational data vocabulary: component, severity, observed timestamp,
  message, and correlation id.
- Internal strategy/risk vocabulary defined by this repo's planning package.

Clean-room rules:

- No `finx-*` dependencies.
- No `tdw-provider-openbb` bridge in v0.1.
- Rust structs remain the source of truth; generated JSON Schema and SQL are
  downstream artifacts.
- Drift evidence is produced by `cargo test -p tdw-domain`, `just schema-sync`,
  and `cargo run -p xtask -- clean-room-audit`.
