#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
}
