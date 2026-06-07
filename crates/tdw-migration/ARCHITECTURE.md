# Architecture — tdw-migration

## Module map

| Path | Contents |
|---|---|
| `src/lib.rs` | `MigrationTarget`, `Migration`, `MigrationCatalogError`, the `*_migrations()` catalog builders, `validate_migration_catalog`, `strip_leading_sql_comments`, `migration_status`, unit tests |

The SQL itself lives outside the crate, under `migrations/postgres/` and
`migrations/clickhouse/`, and is embedded at compile time via `include_str!`.

## Catalog contract & invariants

A [`Migration`] is `{ target, version, name, sql }` (all `&'static str` plus the
`MigrationTarget` enum: `Postgres` | `ClickHouse`). `validate_migration_catalog`
enforces, per target:

- non-empty `version` and `name`,
- non-empty `sql`,
- the **first real statement is `create …`** — `strip_leading_sql_comments` skips
  any leading `--` comment block so a migration may open with an explanatory
  header and still pass,
- **no duplicate `(target, version)`** — caught via a `BTreeSet`, surfaced as
  `DuplicateVersion`.

Versions are date-prefixed (`YYYYMMDD_NNNN`) and the catalog order is the apply
order. The `migrations_cover_required_schema_boundaries` unit test additionally
asserts the Postgres SQL declares the seven warehouse schemas and a long list of
required tables, locking the catalog against accidental removal.

## Migration inventory

### Postgres (`postgres_migrations()`)

| Version | Name | Creates |
|---|---|---|
| `20260521_0001` | `init_schemas` | schemas `raw`, `staging`, `analytics`, `marts`, `agents`, `evals`, `system` |
| `20260521_0002` | `bronze_market_data` | `raw.market_data_bar` |
| `20260521_0003` | `agents_and_evals` | eval-run + related tables |
| `20260521_0004` | `agent_runtime` | `agents.agent_card`, `agent_skill`, `workflow_definition`, `gotcha`, … |
| `20260521_0005` | `event_spine` | `system.event_archive`, `event_outbox`, `event_hook`, `event_replay_run` |
| `20260521_0006` | `parity_layer` | `system.snapshot_version`, `stage_location`, `pipe_definition`, `table_manifest`, … |
| `20260521_0007` | `kg_tags_feature_store` | `system.kg_entity`, `kg_relationship`, `kg_merge_audit`, `tag_*`, `feature_snapshot` |
| `20260521_0008` | `worker_queue` | `system.worker_jobs` |
| `20260528_0001` | `reference_master` | reference-master tables (`ref.*`) |
| `20260528_0002` | `symbol_history` | symbol-history table |
| `20260528_0003` | `trading_calendar` | `ref.trading_calendar` |

### ClickHouse (`clickhouse_migrations()`)

| Version | Name | Creates |
|---|---|---|
| `20260521_0001` | `init_databases` | warehouse databases |
| `20260521_0002` | `bronze_ohlcv` | bronze OHLCV table |
| `20260528_0001` | `raw_equity_historical` | `raw.equity_historical` |
| `20260528_0002` | `raw_tick_trade` | raw tick-trade table |
| `20260528_0003` | `analytics_ohlc_mv` | OHLC materialized view |
| `20260528_0004` | `analytics_stats_mv` | stats materialized view |
| `20260528_0005` | `reference_dictionaries` | reference dictionaries |
| `20260528_0006` | `kafka_ingest` | Kafka table engine + ingest MV (JSONEachRow) |
| `20260528_0007` | `silver_market_data_bar_mv` | silver `market_data_bar` MV |
| `20260528_0008` | `reference_symbol_info` | symbol-info reference table |
| `20260528_0009` | `symbol_dictionaries` | symbol dictionaries |
| `20260528_0010` | `raw_book` | raw order-book table |
| `20260528_0011` | `trading_calendar_dict` | trading-calendar dictionary |
| `20260528_0012` | `analytics_book_stats_mv` | book-stats MV |
| `20260528_0013` | `raw_fundamentals_news` | raw fundamentals/news tables |
| `20260528_0014` | `corporate_actions` | corporate-actions table |
| `20260528_0015` | `analytics_indicators` | indicators analytics |
| `20260528_0016` | `fx_rates` | FX-rates table |
| `20260528_0017` | `analytics_rsi_wilder` | Wilder RSI analytics |
| `20260528_0018` | `analytics_total_return` | total-return analytics |
| `20260528_0019` | `analytics_rolling_vol_fixed_n` | fixed-N rolling-volatility analytics |
| `20260528_0020` | `analytics_rsi_wilder_exact_udf` | exact Wilder RSI UDF analytics |

(The authoritative DDL for each row is the corresponding file under
`migrations/<target>/`; the "Creates" column summarises its leading objects.)

## Real-vs-stub duality

Not applicable — there is no engine and no network. The catalog is identical in
every profile; only the runner's connection target changes.

## Env-gated integration test pattern

Not applicable. The catalog is validated entirely offline by the in-crate unit
tests (`migrations_cover_required_schema_boundaries`,
`rejects_duplicate_migration_versions_per_target`) under the default workspace
test set. Actually *applying* the migrations against live databases is the
`xtask migrate` runner's job, exercised in the CI integration stack.

## Migration story

This crate **is** the migration story for the warehouse. New schema is added by
dropping a `YYYYMMDD_NNNN_name.sql` file under the right `migrations/<target>/`
directory and appending a `Migration { … include_str!(…) }` entry to the matching
`*_migrations()` list; `validate_migration_catalog` then guards ordering/dedup and
the first-statement-is-`create` rule.
