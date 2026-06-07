//! Offline `tdw-sql-codegen` example: generate bronze DDL for both warehouse
//! targets from the domain spec, then emit the idempotent OHLC analytics stack.
//!
//! Run with:
//!
//! ```text
//! cargo run -p tdw-sql-codegen --example tdw-sql-codegen-basic
//! ```

use tdw_sql_codegen::analytics::{default_granularities, emit_ohlc_ddl};
use tdw_sql_codegen::{SqlTarget, export_domain_ddl};

fn main() {
    // Meaningful operation: generate target-specific bronze DDL from the spec.
    let postgres = export_domain_ddl(SqlTarget::Postgres);
    let clickhouse = export_domain_ddl(SqlTarget::ClickHouse);

    println!(
        "Postgres DDL first line: {}",
        postgres.lines().next().unwrap_or("")
    );
    println!(
        "Postgres has MergeTree? {} | ClickHouse has MergeTree? {}",
        postgres.contains("MergeTree"),
        clickhouse.contains("MergeTree")
    );

    // Generate the OHLC analytics stack and count the materialized views.
    let ohlc = emit_ohlc_ddl(default_granularities());
    let mv_count = ohlc
        .matches("create materialized view if not exists")
        .count();
    println!(
        "OHLC stack: {} granularities -> {} materialized view(s)",
        default_granularities().len(),
        mv_count
    );
    println!(
        "uses AggregatingMergeTree: {}",
        ohlc.contains("engine = AggregatingMergeTree")
    );
}
