#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
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

#[must_use]
pub fn postgres_migrations() -> Vec<Migration> {
    let mut migrations = postgres_migrations_core();
    migrations.extend(postgres_migrations_reference());
    migrations
}

/// First half of the Postgres migration catalog (schema init through worker queue).
fn postgres_migrations_core() -> Vec<Migration> {
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
        Migration {
            target: MigrationTarget::Postgres,
            version: "20260521_0008",
            name: "worker_queue",
            sql: include_str!("../../../migrations/postgres/20260521_0008_worker_queue.sql"),
        },
    ]
}

/// Second half of the Postgres migration catalog (reference master through identity tokens).
fn postgres_migrations_reference() -> Vec<Migration> {
    vec![
        Migration {
            target: MigrationTarget::Postgres,
            version: "20260528_0001",
            name: "reference_master",
            sql: include_str!("../../../migrations/postgres/20260528_0001_reference_master.sql"),
        },
        Migration {
            target: MigrationTarget::Postgres,
            version: "20260528_0002",
            name: "symbol_history",
            sql: include_str!("../../../migrations/postgres/20260528_0002_symbol_history.sql"),
        },
        Migration {
            target: MigrationTarget::Postgres,
            version: "20260528_0003",
            name: "trading_calendar",
            sql: include_str!("../../../migrations/postgres/20260528_0003_trading_calendar.sql"),
        },
        Migration {
            target: MigrationTarget::Postgres,
            version: "20260607_0001",
            name: "price_alerts",
            sql: include_str!("../../../migrations/postgres/20260607_0001_price_alerts.sql"),
        },
        Migration {
            target: MigrationTarget::Postgres,
            version: "20260607_0002",
            name: "function_steps",
            sql: include_str!("../../../migrations/postgres/20260607_0002_function_steps.sql"),
        },
        Migration {
            target: MigrationTarget::Postgres,
            version: "20260607_0003",
            name: "identity_users",
            sql: include_str!("../../../migrations/postgres/20260607_0003_identity_users.sql"),
        },
        Migration {
            target: MigrationTarget::Postgres,
            version: "20260608_0001",
            name: "identity_sessions",
            sql: include_str!("../../../migrations/postgres/20260608_0001_identity_sessions.sql"),
        },
        Migration {
            target: MigrationTarget::Postgres,
            version: "20260608_0002",
            name: "identity_reset_tokens",
            sql: include_str!(
                "../../../migrations/postgres/20260608_0002_identity_reset_tokens.sql"
            ),
        },
    ]
}

#[must_use]
pub fn clickhouse_migrations() -> Vec<Migration> {
    let mut migrations = clickhouse_migrations_core();
    migrations.extend(clickhouse_migrations_analytics());
    migrations
}

/// First half of the `ClickHouse` migration catalog (database init through raw book).
fn clickhouse_migrations_core() -> Vec<Migration> {
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
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0001",
            name: "raw_equity_historical",
            sql: include_str!(
                "../../../migrations/clickhouse/20260528_0001_raw_equity_historical.sql"
            ),
        },
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0002",
            name: "raw_tick_trade",
            sql: include_str!("../../../migrations/clickhouse/20260528_0002_raw_tick_trade.sql"),
        },
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0003",
            name: "analytics_ohlc_mv",
            sql: include_str!("../../../migrations/clickhouse/20260528_0003_analytics_ohlc_mv.sql"),
        },
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0004",
            name: "analytics_stats_mv",
            sql: include_str!(
                "../../../migrations/clickhouse/20260528_0004_analytics_stats_mv.sql"
            ),
        },
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0005",
            name: "reference_dictionaries",
            sql: include_str!(
                "../../../migrations/clickhouse/20260528_0005_reference_dictionaries.sql"
            ),
        },
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0006",
            name: "kafka_ingest",
            sql: include_str!("../../../migrations/clickhouse/20260528_0006_kafka_ingest.sql"),
        },
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0007",
            name: "silver_market_data_bar_mv",
            sql: include_str!(
                "../../../migrations/clickhouse/20260528_0007_silver_market_data_bar_mv.sql"
            ),
        },
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0008",
            name: "reference_symbol_info",
            sql: include_str!(
                "../../../migrations/clickhouse/20260528_0008_reference_symbol_info.sql"
            ),
        },
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0009",
            name: "symbol_dictionaries",
            sql: include_str!(
                "../../../migrations/clickhouse/20260528_0009_symbol_dictionaries.sql"
            ),
        },
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0010",
            name: "raw_book",
            sql: include_str!("../../../migrations/clickhouse/20260528_0010_raw_book.sql"),
        },
    ]
}

/// Second half of the `ClickHouse` migration catalog (trading calendar through analytics UDFs).
fn clickhouse_migrations_analytics() -> Vec<Migration> {
    vec![
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0011",
            name: "trading_calendar_dict",
            sql: include_str!(
                "../../../migrations/clickhouse/20260528_0011_trading_calendar_dict.sql"
            ),
        },
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0012",
            name: "analytics_book_stats_mv",
            sql: include_str!(
                "../../../migrations/clickhouse/20260528_0012_analytics_book_stats_mv.sql"
            ),
        },
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0013",
            name: "raw_fundamentals_news",
            sql: include_str!(
                "../../../migrations/clickhouse/20260528_0013_raw_fundamentals_news.sql"
            ),
        },
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0014",
            name: "corporate_actions",
            sql: include_str!("../../../migrations/clickhouse/20260528_0014_corporate_actions.sql"),
        },
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0015",
            name: "analytics_indicators",
            sql: include_str!(
                "../../../migrations/clickhouse/20260528_0015_analytics_indicators.sql"
            ),
        },
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0016",
            name: "fx_rates",
            sql: include_str!("../../../migrations/clickhouse/20260528_0016_fx_rates.sql"),
        },
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0017",
            name: "analytics_rsi_wilder",
            sql: include_str!(
                "../../../migrations/clickhouse/20260528_0017_analytics_rsi_wilder.sql"
            ),
        },
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0018",
            name: "analytics_total_return",
            sql: include_str!(
                "../../../migrations/clickhouse/20260528_0018_analytics_total_return.sql"
            ),
        },
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0019",
            name: "analytics_rolling_vol_fixed_n",
            sql: include_str!(
                "../../../migrations/clickhouse/20260528_0019_analytics_rolling_vol_fixed_n.sql"
            ),
        },
        Migration {
            target: MigrationTarget::ClickHouse,
            version: "20260528_0020",
            name: "analytics_rsi_wilder_exact_udf",
            sql: include_str!(
                "../../../migrations/clickhouse/20260528_0020_analytics_rsi_wilder_exact_udf.sql"
            ),
        },
    ]
}

#[must_use]
pub fn all_migrations() -> Vec<Migration> {
    postgres_migrations()
        .into_iter()
        .chain(clickhouse_migrations())
        .collect()
}

/// Return `sql` with any leading whitespace and `--` line comments removed, so
/// the first real statement can be inspected. Migrations are allowed to open
/// with an explanatory comment block; this lets the catalog validator still
/// confirm the first statement is a `create`.
fn strip_leading_sql_comments(sql: &str) -> &str {
    let mut rest = sql.trim_start();
    while let Some(after) = rest.strip_prefix("--") {
        match after.find('\n') {
            Some(idx) => rest = after[idx + 1..].trim_start(),
            None => return "",
        }
    }
    rest
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
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
        if !strip_leading_sql_comments(sql).starts_with("create ") {
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

#[must_use]
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
            "system.worker_jobs",
            "system.identity_users",
            "system.identity_sessions",
            "system.identity_reset_tokens",
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

    #[test]
    fn catalog_validation_covers_each_boundary_error() {
        let base = Migration {
            target: MigrationTarget::Postgres,
            version: "20260101_0001",
            name: "ok",
            sql: "create table x ();",
        };

        assert_eq!(
            validate_migration_catalog(&[Migration {
                version: "  ",
                ..base
            }]),
            Err(MigrationCatalogError::EmptyVersion {
                target: MigrationTarget::Postgres
            })
        );
        assert_eq!(
            validate_migration_catalog(&[Migration { name: " ", ..base }]),
            Err(MigrationCatalogError::EmptyName {
                target: MigrationTarget::Postgres,
                version: "20260101_0001"
            })
        );
        assert_eq!(
            validate_migration_catalog(&[Migration { sql: "   ", ..base }]),
            Err(MigrationCatalogError::EmptySql {
                target: MigrationTarget::Postgres,
                version: "20260101_0001"
            })
        );
        assert_eq!(
            validate_migration_catalog(&[Migration {
                sql: "insert into x values (1);",
                ..base
            }]),
            Err(MigrationCatalogError::NonCreateSql {
                target: MigrationTarget::Postgres,
                version: "20260101_0001"
            })
        );

        // A leading `--` comment block is stripped before the `create` check.
        assert!(
            validate_migration_catalog(&[Migration {
                sql: "-- header comment\n-- more\ncreate table x ();",
                ..base
            }])
            .is_ok()
        );
    }
}
