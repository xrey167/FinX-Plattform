//! Idempotent DDL emission for the always-fresh "`FlowField`" analytics surfaces.
//!
//! These helpers emit `ClickHouse` DDL strings (idempotent `create ... if not
//! exists`) for the multi-granularity OHLC stack: for each granularity an
//! `AggregatingMergeTree` target, an incremental materialized view that folds
//! `raw.tick` into partial aggregate states, and a reader view that merges
//! those states into plain OHLC numbers.
//!
//! This mirrors the hand-written migration
//! `migrations/clickhouse/20260528_0003_analytics_ohlc_mv.sql`; it exists so
//! the same DDL can be generated/asserted in Rust and is the offline-testable
//! counterpart to the integration-deferred SQL migrations.

/// A candle granularity: its object-name suffix and the matching `ClickHouse`
/// `INTERVAL` unit used by `toStartOfInterval`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Granularity {
    /// Object-name suffix, e.g. `"1m"` -> `analytics.ohlc_1m`.
    pub suffix: &'static str,
    /// Interval magnitude, e.g. `5` in `INTERVAL 5 MINUTE`.
    pub interval_count: u32,
    /// Interval unit keyword, e.g. `"MINUTE"`, `"HOUR"`, `"DAY"`.
    pub interval_unit: &'static str,
    /// Tunable retention TTL expression on `window_start`, e.g.
    /// `"INTERVAL 180 DAY"`. `None` retains indefinitely (e.g. 1d candles).
    pub ttl: Option<&'static str>,
}

/// The default OHLC granularity set: 1m, 5m, 1h, 1d. TTLs are tunable retention
/// defaults mirroring migration 0003 (1d retained indefinitely).
#[must_use]
pub fn default_granularities() -> &'static [Granularity] {
    const G: &[Granularity] = &[
        Granularity {
            suffix: "1m",
            interval_count: 1,
            interval_unit: "MINUTE",
            ttl: Some("INTERVAL 180 DAY"),
        },
        Granularity {
            suffix: "5m",
            interval_count: 5,
            interval_unit: "MINUTE",
            ttl: Some("INTERVAL 1 YEAR"),
        },
        Granularity {
            suffix: "1h",
            interval_count: 1,
            interval_unit: "HOUR",
            ttl: Some("INTERVAL 2 YEAR"),
        },
        Granularity {
            suffix: "1d",
            interval_count: 1,
            interval_unit: "DAY",
            ttl: None,
        },
    ];
    G
}

/// Emit the target table + incremental MV + reader view for one granularity.
#[must_use]
pub fn emit_ohlc_granularity(g: Granularity) -> String {
    let s = g.suffix;
    let count = g.interval_count;
    let unit = g.interval_unit;
    // Optional, tunable retention TTL on window_start. None (e.g. 1d) retains
    // indefinitely, so the target table closes directly after `order by`.
    let ttl_clause = match g.ttl {
        Some(ttl) => format!("\nttl window_start + {ttl}"),
        None => String::new(),
    };
    // non_replicated_deduplication_window lets a retried ingest batch carrying the
    // same insert_deduplication_token (with deduplicate_blocks_in_dependent_
    // materialized_views=1 on the INSERT) dedup THROUGH this MV target, preventing
    // double-counted volume; production ReplicatedMergeTree has this on by default.
    let settings_clause = "\nsettings non_replicated_deduplication_window = 1000";
    format!(
        "create table if not exists analytics.ohlc_{s} (\n  \
           symbol String,\n  \
           venue LowCardinality(String),\n  \
           window_start DateTime,\n  \
           open_state AggregateFunction(argMin, Float64, DateTime64(9)),\n  \
           close_state AggregateFunction(argMax, Float64, DateTime64(9)),\n  \
           high_state AggregateFunction(max, Float64),\n  \
           low_state AggregateFunction(min, Float64),\n  \
           volume_state AggregateFunction(sum, Float64)\n\
         ) engine = AggregatingMergeTree\n\
         partition by toYYYYMM(window_start)\n\
         order by (symbol, venue, window_start){ttl_clause}{settings_clause};\n\n\
         create materialized view if not exists analytics.ohlc_{s}_mv\n\
         to analytics.ohlc_{s}\n\
         as select\n  \
           symbol,\n  \
           venue,\n  \
           toStartOfInterval(ts, INTERVAL {count} {unit}) as window_start,\n  \
           argMinState(price, ts) as open_state,\n  \
           argMaxState(price, ts) as close_state,\n  \
           maxState(price) as high_state,\n  \
           minState(price) as low_state,\n  \
           sumState(size) as volume_state\n\
         from raw.tick\n\
         group by symbol, venue, window_start;\n\n\
         create view if not exists analytics.ohlc_{s}_v\n\
         as select\n  \
           symbol,\n  \
           venue,\n  \
           window_start,\n  \
           argMinMerge(open_state) as open,\n  \
           argMaxMerge(close_state) as close,\n  \
           maxMerge(high_state) as high,\n  \
           minMerge(low_state) as low,\n  \
           sumMerge(volume_state) as volume\n\
         from analytics.ohlc_{s}\n\
         group by symbol, venue, window_start;\n"
    )
}

/// Emit the full OHLC DDL stack for every granularity in `granularities`,
/// concatenated in order with a blank line between granularities.
#[must_use]
pub fn emit_ohlc_ddl(granularities: &[Granularity]) -> String {
    let mut out = String::new();
    for (i, g) in granularities.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&emit_ohlc_granularity(*g));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_aggregating_merge_tree_target() {
        let sql = emit_ohlc_ddl(default_granularities());
        assert!(sql.contains("engine = AggregatingMergeTree"));
        assert!(sql.contains("create table if not exists analytics.ohlc_1m"));
        assert!(sql.contains("create table if not exists analytics.ohlc_1d"));
        assert!(sql.contains("partition by toYYYYMM(window_start)"));
        assert!(sql.contains("order by (symbol, venue, window_start)"));
    }

    #[test]
    fn emits_low_cardinality_and_datetime64_states() {
        let sql = emit_ohlc_ddl(default_granularities());
        // symbol is plain String (high-cardinality universe, leading sort key);
        // it must NOT be LowCardinality. venue stays LowCardinality(String).
        assert!(sql.contains("symbol String"));
        assert!(!sql.contains("symbol LowCardinality(String)"));
        assert!(sql.contains("venue LowCardinality(String)"));
        assert!(sql.contains("open_state AggregateFunction(argMin, Float64, DateTime64(9))"));
        assert!(sql.contains("close_state AggregateFunction(argMax, Float64, DateTime64(9))"));
        // parseDateTimeBestEffort must be fully gone: ts is now a typed column.
        assert!(!sql.contains("parseDateTimeBestEffort"));
    }

    #[test]
    fn emits_per_granularity_ttl_and_retains_1d() {
        let sql = emit_ohlc_ddl(default_granularities());
        assert!(sql.contains("ttl window_start + INTERVAL 180 DAY"));
        assert!(sql.contains("ttl window_start + INTERVAL 1 YEAR"));
        assert!(sql.contains("ttl window_start + INTERVAL 2 YEAR"));
        // 1d retains indefinitely: its target table has no TTL clause and closes
        // with the dedup-window settings clause directly after the order-by.
        let only_1d = [default_granularities()[3]];
        let day = emit_ohlc_ddl(&only_1d);
        assert!(!day.contains("ttl window_start"));
        assert!(day.contains(
            "order by (symbol, venue, window_start)\nsettings non_replicated_deduplication_window = 1000;"
        ));
    }

    #[test]
    fn emits_dedup_window_on_every_target() {
        let sql = emit_ohlc_ddl(default_granularities());
        // Each AggregatingMergeTree target carries the dedup window so a retried
        // ingest batch dedups THROUGH the MV (no double-counted volume).
        assert_eq!(
            sql.matches("settings non_replicated_deduplication_window = 1000")
                .count(),
            default_granularities().len()
        );
        // With a TTL present (e.g. 1m), settings comes AFTER the ttl clause.
        assert!(sql.contains(
            "ttl window_start + INTERVAL 180 DAY\nsettings non_replicated_deduplication_window = 1000;"
        ));
    }

    #[test]
    fn emits_arg_min_state_and_merge() {
        let sql = emit_ohlc_ddl(default_granularities());
        assert!(sql.contains("argMinState(price, ts) as open_state"));
        assert!(sql.contains("argMaxState(price, ts) as close_state"));
        assert!(sql.contains("argMinMerge(open_state) as open"));
        assert!(sql.contains("argMaxMerge(close_state) as close"));
    }

    #[test]
    fn emits_to_start_of_interval_per_unit() {
        let sql = emit_ohlc_ddl(default_granularities());
        assert!(sql.contains("toStartOfInterval(ts, INTERVAL 1 MINUTE)"));
        assert!(sql.contains("toStartOfInterval(ts, INTERVAL 5 MINUTE)"));
        assert!(sql.contains("toStartOfInterval(ts, INTERVAL 1 HOUR)"));
        assert!(sql.contains("toStartOfInterval(ts, INTERVAL 1 DAY)"));
    }

    #[test]
    fn emits_materialized_view_to_target() {
        let sql = emit_ohlc_ddl(default_granularities());
        assert!(sql.contains("create materialized view if not exists analytics.ohlc_1m_mv"));
        assert!(sql.contains("to analytics.ohlc_1m"));
        // The MV must target the analytics database explicitly.
        assert!(sql.contains("create materialized view if not exists analytics.ohlc_1d_mv"));
        assert!(sql.contains("to analytics.ohlc_1d"));
    }

    #[test]
    fn emits_reader_view_merging_states() {
        let sql = emit_ohlc_ddl(default_granularities());
        assert!(sql.contains("create view if not exists analytics.ohlc_1m_v"));
        assert!(sql.contains("maxMerge(high_state) as high"));
        assert!(sql.contains("minMerge(low_state) as low"));
        assert!(sql.contains("sumMerge(volume_state) as volume"));
    }

    #[test]
    fn emission_is_idempotent() {
        assert_eq!(
            emit_ohlc_ddl(default_granularities()),
            emit_ohlc_ddl(default_granularities())
        );
    }

    #[test]
    fn respects_supplied_granularity_subset() {
        let only_1m = [default_granularities()[0]];
        let sql = emit_ohlc_ddl(&only_1m);
        assert!(sql.contains("analytics.ohlc_1m"));
        assert!(!sql.contains("analytics.ohlc_5m"));
        assert!(!sql.contains("analytics.ohlc_1d"));
    }
}
