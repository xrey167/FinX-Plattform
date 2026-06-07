# tdw-domain

Canonical, validated domain types for the trading data warehouse: the "bill of
materials" (BOM) of market-data, order, position, reference-data and analytics
records that every other crate ingests, validates and emits.

## Purpose

`tdw-domain` is the single source of truth for the platform's business objects.
Each type is a plain `serde`-serializable struct or enum that also derives:

- [`schemars::JsonSchema`] so the same shape can be exported as JSON Schema for
  cross-language consumers and registry checks;
- [`validator::Validate`] so a constructed value can be checked (`row.validate()`)
  against field-level invariants (non-empty strings, non-negative prices, bounded
  sentiment scores, fixed-width identifiers).

It also exposes fixed-width and non-empty **reference-id newtypes**
(`Figi`, `Isin`, `Cusip`, `Sedol`, `Mic`, `CountryAlpha2`, `CurrencyCode`,
`IssuerId`, `ClassificationCode`) whose constructors reject malformed ids up
front, and the [`BOM_SCHEMA_NAMES`] constant naming the 11 BOM schema families.

The crate is pure data: `#![forbid(unsafe_code)]`, no I/O, no async.

## Feature flags

None. The crate has no optional features and no Cargo features are defined.

## Dependencies

- `serde` — (de)serialization
- `schemars` — JSON Schema generation
- `validator` — declarative field validation

## Quickstart

```rust
use tdw_domain::{EquityHistoricalData, Figi};
use validator::Validate;

// Construct a domain row and validate its invariants.
let bar = EquityHistoricalData {
    symbol: "AAPL".to_string(),
    date: "2026-05-21".to_string(),
    open: 100.0,
    high: 101.0,
    low: 99.0,
    close: 100.5,
    volume: 1_000,
};
assert!(bar.validate().is_ok());

// Reference-id newtypes reject malformed identifiers at construction.
let figi = Figi::new("BBG000B9XRY4").expect("12-char FIGI");
assert_eq!(figi.as_str(), "BBG000B9XRY4");
assert!(Figi::new("TOO_SHORT").is_err());
```

Run the worked example:

```text
cargo run -p tdw-domain --example basic
```

## See also

- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — type map, validation model, invariants.
- `tdw-sql-codegen` — emits DDL annotated with the BOM schema count.
