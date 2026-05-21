#![forbid(unsafe_code)]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlTarget {
    Postgres,
    ClickHouse,
}

pub fn export_market_data_bar(target: SqlTarget) -> &'static str {
    match target {
        SqlTarget::Postgres => include_str!("../../../sql/ddl/postgres_bronze.sql"),
        SqlTarget::ClickHouse => include_str!("../../../sql/ddl/clickhouse_bronze.sql"),
    }
}

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
}
