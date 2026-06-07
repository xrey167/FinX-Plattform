# tdw-migration

Embedded SQL migration **catalog** for the FinX data-warehouse (Postgres +
ClickHouse).

## Purpose

`tdw-migration` is the single source of truth for the warehouse schema. It
`include_str!`s every `.sql` file under `migrations/postgres/` and
`migrations/clickhouse/` into a typed, validated catalog of [`Migration`]
records, so the migration runner (`xtask migrate`) and tests can enumerate and
verify migrations without a database connection.

- `postgres_migrations()` — the ordered Postgres migration list.
- `clickhouse_migrations()` — the ordered ClickHouse migration list.
- `all_migrations()` — both, concatenated.
- `validate_migration_catalog(&[Migration])` — shape + ordering + dedup checks.
- `migration_status()` — `"postgres=N clickhouse=M"` summary string.

## Engine trait

None. This crate implements **no** `tdw_core` engine trait and opens no
connection. It is a compile-time-embedded catalog of SQL text; applying it is the
job of the runner (which uses the relational/OLAP engines).

## Default vs real backend

Not applicable — no backend, no feature flag, no network. The SQL is baked into
the binary at compile time via `include_str!`; depends only on `serde` +
`thiserror`.

## Connection / env vars

None. The catalog is data; the runner (`cargo run -p xtask -- migrate up`) owns
the connection and credentials.

## `TDW_PROFILE=live` behavior

None directly. The same catalog is applied in any profile; in the `live` profile
the runner targets the live Postgres / ClickHouse, but the migration *list* is
profile-independent.

## Quickstart

```rust
use tdw_migration::{
    all_migrations, clickhouse_migrations, migration_status, postgres_migrations,
    validate_migration_catalog,
};

let pg = postgres_migrations();
let ch = clickhouse_migrations();

validate_migration_catalog(&pg).expect("postgres catalog is valid");
validate_migration_catalog(&ch).expect("clickhouse catalog is valid");

assert_eq!(all_migrations().len(), pg.len() + ch.len());
println!("{}", migration_status()); // e.g. "postgres=11 clickhouse=22"
```

```sh
cargo run -p tdw-migration --example tdw-migration-basic
```

See [`ARCHITECTURE.md`](ARCHITECTURE.md) for the full migration-file inventory and
what each creates.
