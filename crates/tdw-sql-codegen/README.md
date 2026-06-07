# tdw-sql-codegen

Deterministic, target-specific SQL/DDL generation for the warehouse. It emits the
bronze DDL for the domain BOM (Postgres or ClickHouse) and the idempotent
ClickHouse OHLC analytics stack (target tables + materialized views + reader
views).

## Purpose

The crate turns the platform's data model into runnable DDL strings:

- `export_market_data_bar(target)` — returns the vendored bronze DDL for a target
  (the Postgres / ClickHouse `.sql` files are `include_str!`-embedded at compile
  time);
- `export_domain_ddl(target)` — the same DDL prefixed with a comment annotating the
  number of `tdw-domain` BOM schemas;
- `analytics::emit_ohlc_ddl(granularities)` — generates the full multi-granularity
  OHLC `AggregatingMergeTree` stack (mirrors migration `…0003_analytics_ohlc_mv`)
  so the same DDL can be asserted in Rust offline.

The crate is pure: `#![forbid(unsafe_code)]`, no I/O at runtime (DDL is embedded or
formatted from constants).

## Feature flags

None.

## Dependencies

- `tdw-domain` — read `BOM_SCHEMA_NAMES.len()` for the DDL annotation.

## Quickstart

```rust
use tdw_sql_codegen::{SqlTarget, export_domain_ddl};
use tdw_sql_codegen::analytics::{default_granularities, emit_ohlc_ddl};

// Bronze DDL for a chosen warehouse target.
let pg = export_domain_ddl(SqlTarget::Postgres);
assert!(pg.contains("create table if not exists raw.market_data_bar"));

let ch = export_domain_ddl(SqlTarget::ClickHouse);
assert!(ch.contains("MergeTree"));

// Idempotent OHLC analytics stack.
let ohlc = emit_ohlc_ddl(default_granularities());
assert!(ohlc.contains("engine = AggregatingMergeTree"));
```

Run the worked example:

```text
cargo run -p tdw-sql-codegen --example tdw-sql-codegen-basic
```

## See also

- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — output contract and the OHLC DDL shape.
- `tdw-domain` — the BOM whose schema count annotates the output.
