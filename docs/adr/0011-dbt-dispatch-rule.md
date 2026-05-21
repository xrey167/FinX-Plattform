# ADR-0011: dbt Dispatch Rule

Status: accepted

Postgres is the default target for relational/reference/agent metadata shapes.
ClickHouse is the default target for OHLCV, tick, eval, and observability shapes.

The dbt project keeps both profiles in `profiles.yml.template`. Per-model tags
declare layer and domain; Rust-generated DDL remains the canonical table source.
SQLMesh can be revisited after the dbt layer is proven.
