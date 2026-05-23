#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, MigrationCatalogError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MigrationTarget {
    Postgres,
    ClickHouse,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Migration {
    pub target: MigrationTarget,
    pub version: &'static str,
    pub name: &'static str,
    pub sql: &'static str,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MigrationCatalogError {
    #[error("migration version must not be empty for {target:?}")]
    EmptyVersion { target: MigrationTarget },
    #[error("migration name must not be empty for {target:?} version {version}")]
    EmptyName {
        target: MigrationTarget,
        version: &'static str,
    },
    #[error("migration sql must not be empty for {target:?} version {version}")]
    EmptySql {
        target: MigrationTarget,
        version: &'static str,
    },
    #[error("migration sql for {target:?} version {version} must start with create")]
    NonCreateSql {
        target: MigrationTarget,
        version: &'static str,
    },
    #[error("duplicate migration version {version} for {target:?}")]
    DuplicateVersion {
        target: MigrationTarget,
        version: &'static str,
    },
}

pub fn postgres_migrations() -> Vec<Migration> {
    vec![
        Migration {
            target: MigrationTarget::Postgres,
            version: "20260521_0001",
            name: "init_schemas",
            sql: include_str!("../../../migrations/postgres/20260521_0001_init_schemas.sql"),
        },
        Migration {
            target: MigrationTarget::Postgres,
            version: "20260521_0002",
            name: "bronze_market_data",
            sql: include_str!("../../../migrations/postgres/20260521_0002_bronze_market_data.sql"),
        },
        Migration {
            target: MigrationTarget::Postgres,
            version: "20260521_0003",
            name: "agents_and_evals",
            sql: include_str!("../../../migrations/postgres/20260521_0003_agents_and_evals.sql"),
        },
        Migration {
            target: MigrationTarget::Postgres,
            version: "20260521_0004",
            name: "agent_runtime",
            sql: include_str!("../../../migrations/postgres/20260521_0004_agent_runtime.sql"),
        },
        Migration {
            target: MigrationTarget::Postgres,
            version: "20260521_0005",
            name: "event_spine",
            sql: include_str!("../../../migrations/postgres/20260521_0005_event_spine.sql"),
        },
        Migration {
            target: MigrationTarget::Postgres,
            version: "20260521_0006",
            name: "parity_layer",
            sql: include_str!("../../../migrations/postgres/20260521_0006_parity_layer.sql"),
        },
        Migration {
            target: MigrationTarget::Postgres,
            version: "20260521_0007",
            name: "kg_tags_feature_store",
            sql: include_str!(
                "../../../migrations/postgres/20260521_0007_kg_tags_feature_store.sql"
            ),
        },
    ]
}

pub fn clickhouse_migrations() -> Vec<Migration> {
    vec![
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260521_0001",
            name: "init_databases",
            sql: include_str!("../../../migrations/clickhouse/20260521_0001_init_databases.sql"),
        },
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260521_0002",
            name: "bronze_ohlcv",
            sql: include_str!("../../../migrations/clickhouse/20260521_0002_bronze_ohlcv.sql"),
        },
    ]
}

pub fn all_migrations() -> Vec<Migration> {
    postgres_migrations()
        .into_iter()
        .chain(clickhouse_migrations())
        .collect()
}

pub fn validate_migration_catalog(migrations: &[Migration]) -> Result<()> {
    let mut versions = BTreeSet::new();
    for migration in migrations {
        if migration.version.trim().is_empty() {
            return Err(MigrationCatalogError::EmptyVersion {
                target: migration.target,
            });
        }
        if migration.name.trim().is_empty() {
            return Err(MigrationCatalogError::EmptyName {
                target: migration.target,
                version: migration.version,
            });
        }
        let sql = migration.sql.trim_start();
        if sql.is_empty() {
            return Err(MigrationCatalogError::EmptySql {
                target: migration.target,
                version: migration.version,
            });
        }
        if !sql.starts_with("create ") {
            return Err(MigrationCatalogError::NonCreateSql {
                target: migration.target,
                version: migration.version,
            });
        }
        if !versions.insert((migration.target, migration.version)) {
            return Err(MigrationCatalogError::DuplicateVersion {
                target: migration.target,
                version: migration.version,
            });
        }
    }
    Ok(())
}

pub fn migration_status() -> String {
    format!(
        "postgres={} clickhouse={}",
        postgres_migrations().len(),
        clickhouse_migrations().len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_cover_required_schema_boundaries() {
        assert!(validate_migration_catalog(&postgres_migrations()).is_ok());
        assert!(validate_migration_catalog(&clickhouse_migrations()).is_ok());
        assert_eq!(
            all_migrations().len(),
            postgres_migrations().len() + clickhouse_migrations().len()
        );

        let postgres_sql = postgres_migrations()
            .iter()
            .map(|migration| migration.sql)
            .collect::<Vec<_>>()
            .join("\n");

        for schema in [
            "raw",
            "staging",
            "analytics",
            "marts",
            "agents",
            "evals",
            "system",
        ] {
            assert!(postgres_sql.contains(&format!("schema if not exists {schema}")));
        }
        for table in [
            "agents.agent_card",
            "agents.agent_skill",
            "agents.workflow_definition",
            "agents.gotcha",
            "evals.eval_metric",
            "system.event_archive",
            "system.event_outbox",
            "system.event_hook",
            "system.event_replay_run",
            "system.snapshot_version",
            "system.stage_location",
            "system.pipe_definition",
            "system.table_manifest",
            "system.udf_definition",
            "system.auth_policy",
            "system.mask_rule",
            "system.define_statement",
            "system.graph_edge",
            "system.spatial_box",
            "system.kg_entity",
            "system.kg_relationship",
            "system.kg_merge_audit",
            "system.tag_definition",
            "system.tag_assignment",
            "system.tag_rule",
            "system.feature_snapshot",
        ] {
            assert!(
                postgres_sql.contains(table),
                "missing migration table: {table}"
            );
        }
    }

    #[test]
    fn rejects_duplicate_migration_versions_per_target() {
        let mut migrations = postgres_migrations();
        migrations.push(migrations[0].clone());

        assert_eq!(
            validate_migration_catalog(&migrations),
            Err(MigrationCatalogError::DuplicateVersion {
                target: MigrationTarget::Postgres,
                version: "20260521_0001",
            })
        );
    }
}
