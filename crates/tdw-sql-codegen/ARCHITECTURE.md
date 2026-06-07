# tdw-sql-codegen — Architecture

## Module map

| File | Contents |
| --- | --- |
| `lib.rs` | `SqlTarget`, bronze DDL export (`export_market_data_bar`, `export_domain_ddl`). |
| `analytics.rs` | Idempotent ClickHouse OHLC DDL stack (`Granularity`, `emit_ohlc_ddl`, …). |

### `lib.rs` items

| Item | Role |
| --- | --- |
| `SqlTarget` | `Postgres`, `ClickHouse`. |
| `export_market_data_bar` | `const fn` returning embedded bronze DDL for a target. |
| `export_domain_ddl` | Bronze DDL prefixed with a BOM-schema-count comment. |

### `analytics.rs` items

| Item | Role |
| --- | --- |
| `Granularity` | Candle spec: `suffix`, `interval_count`, `interval_unit`, `ttl`. |
| `default_granularities` | `const` set: 1m, 5m, 1h, 1d. |
| `emit_ohlc_granularity` | One granularity → target + MV + reader view DDL. |
| `emit_ohlc_ddl` | Concatenate the per-granularity DDL in order. |

## Output contract

The crate's product is **deterministic SQL strings**. Two contracts matter:

1. **Bronze DDL.** `export_market_data_bar` is a `const fn` that returns the
   `include_str!`-embedded DDL file for the target — Postgres DDL has no
   `MergeTree`; ClickHouse DDL has `engine = MergeTree` and `order by (symbol, ts)`.
   `export_domain_ddl` prepends `-- generated from <N> tdw-domain BOM schemas`,
   where `N = tdw_domain::BOM_SCHEMA_NAMES.len()`, then the bronze DDL.

2. **OHLC analytics stack.** For each `Granularity`, `emit_ohlc_granularity`
   produces three objects, all `create ... if not exists` (idempotent):
   - `analytics.ohlc_<suffix>` — `AggregatingMergeTree` of partial aggregate
     states (`argMinState`/`argMaxState`/`maxState`/`minState`/`sumState`),
     partitioned `by toYYYYMM(window_start)`, ordered `(symbol, venue,
     window_start)`, with an optional per-granularity TTL and a dedup-window
     setting;
   - `analytics.ohlc_<suffix>_mv` — incremental materialized view folding
     `raw.tick` into those states via `toStartOfInterval(ts, INTERVAL n unit)`;
   - `analytics.ohlc_<suffix>_v` — reader view merging the states
     (`argMinMerge`/…/`sumMerge`) into plain OHLCV columns.

## Data flow

```
SqlTarget ──▶ export_market_data_bar (include_str! at compile time) ──▶ &'static str
                          │
tdw_domain::BOM_SCHEMA_NAMES.len() ──▶ export_domain_ddl ──▶ annotated String

&[Granularity] ──▶ emit_ohlc_ddl ──▶ per-granularity {target, MV, reader view} String
```

## Invariants

- **Idempotent emission.** `export_domain_ddl(t)` and `emit_ohlc_ddl(g)` return the
  same string on repeated calls; every emitted statement is `create … if not
  exists`.
- **Target separation.** Postgres output never contains `MergeTree`; ClickHouse
  output contains `engine = MergeTree` (bronze) / `AggregatingMergeTree` (OHLC).
- **`symbol` stays plain `String`** (high-cardinality leading sort key); `venue` is
  `LowCardinality(String)`. Timestamps are typed `DateTime64(9)` states — no
  `parseDateTimeBestEffort`.
- **Per-granularity TTL** is optional: `ttl: None` (e.g. 1d) retains indefinitely
  and the target table closes directly with the dedup-window settings clause.
- The dedup-window setting (`non_replicated_deduplication_window = 1000`) appears
  on **every** target so retried ingest batches dedup through the MV.
- The emitted DDL mirrors the hand-written migration `…0003_analytics_ohlc_mv.sql`,
  keeping the Rust generator and the SQL migration in lockstep.
