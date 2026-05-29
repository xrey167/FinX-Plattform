-- Canonical three-tier broker -> ClickHouse consumer for the optional Kafka
-- ingest path (tdw-storage-broker, `ingest-broker` feature). The Rust producer
-- writes one JSONEachRow message per row to the `tdw.tick` topic; this file
-- defines the ClickHouse side that drains that topic into raw.tick.
--
-- Three tiers:
--   1. analytics.tick_queue  -- engine = Kafka, the consuming queue
--   2. raw.tick              -- the durable target (REUSED from migration 0002,
--                               a *MergeTree with insert deduplication enabled)
--   3. analytics.tick_queue_mv -- materialized view: queue -> raw.tick
--
-- DELIVERY SEMANTICS (important):
-- The ClickHouse Kafka table engine is AT-LEAST-ONCE with a DESTRUCTIVE consume:
-- reading a block advances the consumer-group offset, so a crash AFTER the read
-- but BEFORE the target write is durably committed can drop or (on retry)
-- duplicate rows. Duplicates are REDUCED -- not eliminated -- by the dedup on
-- the raw.tick target (ReplicatedMergeTree block dedup in production, or the
-- non_replicated_deduplication_window set in migration 0002 for single-node).
--
-- EXACTLY-ONCE is NOT achievable with the table engine. The documented upgrade
-- path is the separate ClickHouse Kafka Connect Sink connector, which tracks
-- consumed offsets in a KeeperMap state table and replays from the last
-- committed offset on restart, giving exactly-once delivery into ClickHouse.
-- That connector runs OUT of the database process and is the production choice
-- when no row loss/duplication is tolerable; the table engine below is the
-- lightweight, in-database default.
--
-- CONFIG: kafka_broker_list below is a PLACEHOLDER. The real bootstrap broker
-- list is supplied via deployment config (the same `brokers` string passed to
-- RskafkaBrokerSink::connect) and must be substituted before applying this
-- migration to a live cluster.

-- `ts` is typed DateTime64(9, 'UTC') to match the raw.tick target (migration
-- 0002). The Kafka engine parses the JSONEachRow ISO-8601 timestamp string
-- directly into DateTime64(9), exactly as the direct INSERT path does, so the
-- producer (RskafkaBrokerSink) keeps emitting ISO strings unchanged.
create table if not exists analytics.tick_queue (
  symbol String,
  venue LowCardinality(String),
  ts DateTime64(9, 'UTC'),
  price Float64,
  size Float64
) engine = Kafka
settings
  kafka_broker_list = 'PLACEHOLDER:9092',
  kafka_topic_list = 'tdw.tick',
  kafka_group_name = 'tdw-clickhouse',
  kafka_format = 'JSONEachRow';

-- Drain the queue into the durable raw.tick target. ingested_at is omitted so
-- raw.tick's `DEFAULT now64(3)` fills it on insert.
create materialized view if not exists analytics.tick_queue_mv
to raw.tick
as select
  symbol,
  venue,
  ts,
  price,
  size
from analytics.tick_queue;
