# ADR-0014: License — Dual MIT OR Apache-2.0

## Status

Supersedes ADR-0009.

## Decision

FinX-Plattform is licensed under either of:

- Apache License, Version 2.0 (`LICENSE-APACHE`)
- MIT license (`LICENSE-MIT`)

at the user's option. This is the Rust ecosystem default (used by `rust-lang/rust`,
the standard library, and the vast majority of crates.io packages). Workspace
crates now declare `license = "MIT OR Apache-2.0"`; `publish = false` is retained
on every crate because none are intended for crates.io publication yet.

Contributions are dual-licensed under the same terms unless the contributor
explicitly states otherwise, per the Apache-2.0 inbound-equals-outbound
convention restated in the README "Contribution" section.

## Consequences

- External contributors and consumers have explicit usage rights under both a
  permissive copyright license (MIT) and an Apache-2.0-style patent grant.
  The earlier `UNLICENSED` framing actively discouraged contribution and use.
- Per-crate `license = "MIT OR Apache-2.0"` is the SPDX identifier consumed by
  `cargo deny check`, `crates.io` (if a crate is ever published), and license
  scanners. No change to the existing `cargo deny check` allowed-license list
  is required: MIT and Apache-2.0 are already in the standard allow list.
- Future relicensing requires consent from every contributor whose work landed
  under MIT OR Apache-2.0. Practically: each PR author becomes a co-licensor.
  This is the same constraint every open-source Rust project operates under.
- `publish = false` is preserved per crate. Going to crates.io is a separate
  decision, gated on stabilising public APIs and naming.
- ADR-0009 ("License And Publication Boundary" — `UNLICENSED` for private
  personal use) is superseded. The historical decision is retained in-tree for
  context.