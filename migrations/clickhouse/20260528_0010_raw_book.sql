-- Raw L2 order-book depth: one row per price level per book snapshot. Each
-- snapshot fans out into N rows -- side = 'bid'/'ask', level = 1..N (level 1 is
-- the best bid/ask) -- carrying that level's price and aggregate size. This is
-- the full-depth counterpart to raw.quote, which remains the top-of-book
-- (level-1) convenience surface. A depth-analytics MV (e.g. book imbalance,
-- cumulative depth, microprice) is a documented follow-up over this table.
--
-- Typing / codec / dedup rationale matches raw.tick/raw.trade/raw.quote
-- (see 20260528_0002): symbol plain String (high-cardinality leading key),
-- venue/side LowCardinality, ts DateTime64(9,'UTC') codec(DoubleDelta, ZSTD),
-- Float64 price/size codec(ZSTD), ingested_at default now64(3). ORDER BY adds
-- (side, level) after (symbol, ts) so the levels of one snapshot stay
-- contiguous on disk and a per-snapshot ladder read is a single range scan.
create table if not exists raw.book_level (
  symbol String,
  venue LowCardinality(String),
  ts DateTime64(9, 'UTC') codec(DoubleDelta, ZSTD),
  side LowCardinality(String),
  level UInt8,
  price Float64 codec(ZSTD),
  size Float64 codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(ts)
order by (symbol, ts, side, level)
ttl toDateTime(ts) + interval 30 day -- tunable: raw book-level retention
settings non_replicated_deduplication_window = 1000;
