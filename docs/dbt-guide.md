# dbt Guide

The dbt project lives in `dbt/finx_finance`.

- Postgres target: `pg_dev`
- ClickHouse target: `ch_dev`
- Rust domain structs remain the source of truth for DDL.
- `cargo run -p xtask -- ddl-export postgres` emits generated Postgres DDL.
- `cargo run -p xtask -- migrate status` reports the offline migration plan.

Live `dbt debug` and `dbt build` require local `dbt`, Docker, Postgres, and
ClickHouse availability.
