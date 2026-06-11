-- Bronze landing table for the CFTC Commitments-of-Traders wave (openbb-parity
-- P2W5). The cftc ingest binding (tdw_service_api::dispatcher::
-- insert_cftc_ingest_bindings, regulators/cftc/cot -> raw.commitment_of_traders)
-- is already on main; this creates the table it writes to so the INSERT ...
-- FORMAT JSONEachRow does not fail at runtime. Columns mirror
-- tdw_domain::CommitmentOfTraders byte-for-byte.
--
-- Typing / codec / dedup rationale matches the 0021-0026 breadth migrations:
-- the high-cardinality identifier (market) leads the ORDER BY as a plain String;
-- report_date is part of the sorting key so it is a non-nullable String
-- (absent dates arrive as '' via input_format_null_as_default); the open-interest
-- and position columns are Nullable(Float64). The non-idempotent ingested_at
-- DEFAULT defeats content-hash dedup, so the ingest path supplies an explicit
-- insert_deduplication_token per batch; non_replicated_deduplication_window lets
-- that token take effect on this non-replicated MergeTree table (swap to
-- ReplicatedMergeTree in production). COT reports are weekly snapshots keyed by
-- (market, report_date); partition by the ingest day.

create table if not exists raw.commitment_of_traders (
  market String,
  -- Non-nullable: part of the sorting key; ClickHouse rejects nullable key
  -- columns. Absent report dates arrive as '' (input_format_null_as_default).
  report_date String default '',
  open_interest Nullable(Float64) codec(ZSTD),
  noncommercial_long Nullable(Float64) codec(ZSTD),
  noncommercial_short Nullable(Float64) codec(ZSTD),
  commercial_long Nullable(Float64) codec(ZSTD),
  commercial_short Nullable(Float64) codec(ZSTD),
  total_reportable_long Nullable(Float64) codec(ZSTD),
  total_reportable_short Nullable(Float64) codec(ZSTD),
  nonreportable_long Nullable(Float64) codec(ZSTD),
  nonreportable_short Nullable(Float64) codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(toDate(ingested_at))
order by (market, report_date)
settings non_replicated_deduplication_window = 1000;
