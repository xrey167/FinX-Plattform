//! Offline migration-catalog round-trip: enumerate the embedded Postgres and
//! `ClickHouse` migrations and validate them. No network, no docker — the catalog
//! is baked into the binary at compile time.
//!
//! Run with: `cargo run -p tdw-migration --example tdw-migration-basic`

use tdw_migration::{
    all_migrations, clickhouse_migrations, migration_status, postgres_migrations,
    validate_migration_catalog,
};

fn main() -> tdw_migration::Result<()> {
    let pg = postgres_migrations();
    let ch = clickhouse_migrations();

    // The catalog must be well-formed: non-empty, create-first, no dup versions.
    validate_migration_catalog(&pg)?;
    validate_migration_catalog(&ch)?;

    assert_eq!(all_migrations().len(), pg.len() + ch.len());

    let first_pg = pg.first().expect("at least one postgres migration");
    println!(
        "catalog ok: {} | first postgres migration = {} ({})",
        migration_status(),
        first_pg.version,
        first_pg.name
    );
    Ok(())
}
