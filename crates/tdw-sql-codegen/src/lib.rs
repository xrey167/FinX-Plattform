#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
pub mod analytics;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlTarget {
    Postgres,
    ClickHouse,
}

#[must_use]
pub const fn export_market_data_bar(target: SqlTarget) -> &'static str {
    match target {
        SqlTarget::Postgres => include_str!("../../../sql/ddl/postgres_bronze.sql"),
        SqlTarget::ClickHouse => include_str!("../../../sql/ddl/clickhouse_bronze.sql"),
    }
}

#[must_use]
pub fn export_domain_ddl(target: SqlTarget) -> String {
    let schema_count = tdw_domain::BOM_SCHEMA_NAMES.len();
    format!(
        "-- generated from {schema_count} tdw-domain BOM schemas\n{}",
        export_market_data_bar(target)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddl_export_is_idempotent() {
        assert_eq!(
            export_domain_ddl(SqlTarget::Postgres),
            export_domain_ddl(SqlTarget::Postgres)
        );
        assert!(export_domain_ddl(SqlTarget::ClickHouse).contains("MergeTree"));
    }

    #[test]
    fn ddl_export_is_target_specific_and_domain_annotated() {
        let postgres = export_domain_ddl(SqlTarget::Postgres);
        let clickhouse = export_domain_ddl(SqlTarget::ClickHouse);

        assert!(postgres.starts_with("-- generated from "));
        assert!(postgres.contains("create table if not exists raw.market_data_bar"));
        assert!(!postgres.contains("MergeTree"));
        assert!(clickhouse.contains("engine = MergeTree"));
        assert!(clickhouse.contains("order by (symbol, ts)"));
    }
}
