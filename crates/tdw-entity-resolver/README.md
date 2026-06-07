# tdw-entity-resolver

Offline entity resolution for instruments: match a ticker or a standardized
identifier (FIGI/ISIN/…) to a knowledge-graph entity, and model audited
manual-merge decisions. Includes a pure parser for `OpenFIGI` mapping responses.

## Purpose

The resolver answers "which instrument is this?" without any database round-trip:

- `resolve_symbol(symbol, &entities)` — exact ticker/alias match against
  `tdw_kg::Entity`s of kind `Instrument` (case-insensitive), scored 100;
- `resolve_by_identifier(scheme, value, &records)` — match against an in-memory
  `IdentifierRecord` crosswalk (mirrors `ref.identifier_xref`);
- `manual_merge_decision(...)` — record an audited merge of two entities;
- `openfigi::parse_openfigi_mapping(json)` — turn an `OpenFIGI` `/v3/mapping`
  response body into typed rows (no network I/O).

Every public function has a `try_*` variant that validates inputs against safe
symbol/identifier/entity-id grammars; the non-`try_` variants return empty/default
on bad input.

The crate is pure: `#![forbid(unsafe_code)]`, no network I/O.

## Feature flags

None.

## Dependencies

- `serde`, `serde_json` — record types and the `OpenFIGI` JSON parser.
- `tdw-kg` — `Entity` / `EntityKind` the symbol resolver matches against.

## Quickstart

```rust
use tdw_entity_resolver::resolve_symbol;
use tdw_kg::{Entity, EntityKind};

let entities = vec![Entity {
    entity_id: "instrument:AAPL".to_string(),
    kind: EntityKind::Instrument,
    label: "Apple".to_string(),
    aliases: vec!["AAPL".to_string()],
}];

let hits = resolve_symbol("aapl", &entities);
assert_eq!(hits[0].entity_id, "instrument:AAPL");
assert_eq!(hits[0].score, 100);
```

Run the worked example:

```text
cargo run -p tdw-entity-resolver --example basic
```

## See also

- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — resolution paths, scoring, merge audit.
- `tdw-kg` — the knowledge-graph entity model resolved against.
