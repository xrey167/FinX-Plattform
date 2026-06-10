-- Bronze landing tables for the keyless government wave (gap-matrix items
-- L2.6 SEC, L3.1 Federal Reserve, L3.2 US Treasury). Rows are written verbatim
-- from the provider row shapes via INSERT ... FORMAT JSONEachRow by the ingest
-- dispatcher, mirroring the tdw_domain row structs (SymbolMapping,
-- OwnershipRecord, EtfHolding, TreasuryAuction, TreasuryPrice, FomcDocument) so
-- the column shapes agree byte-for-byte.
--
-- Typing / codec / dedup rationale matches 20260528_0021_raw_fred_macro_rate:
-- high-cardinality identifiers are plain String leading keys; low-cardinality
-- descriptive columns use LowCardinality; numeric value columns are
-- Nullable(Float64) because the upstream sources frequently omit fields. The
-- non-idempotent ingested_at DEFAULT defeats content-hash dedup, so the ingest
-- path supplies an explicit insert_deduplication_token per batch; the
-- non_replicated_deduplication_window below lets that token take effect on these
-- non-replicated MergeTree tables (swap to ReplicatedMergeTree in production).

-- raw.symbol_mapping mirrors tdw_domain::SymbolMapping: one (symbol, cik) pair
-- from SEC company_tickers.json. No event date; ORDER BY (symbol, cik) keeps the
-- directory deterministic. No TTL (a durable, small symbology directory).
create table if not exists raw.symbol_mapping (
  symbol String,
  cik String,
  name Nullable(String),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
order by (symbol, cik)
settings non_replicated_deduplication_window = 1000;

-- raw.ownership_record mirrors tdw_domain::OwnershipRecord: the equity ownership
-- cluster (form_13f filing index, fails_to_deliver). `kind` discriminates the
-- record variant; most numeric fields are Nullable since each endpoint reports a
-- different subset. ORDER BY (symbol, kind, date) keeps a symbol's records
-- contiguous; `date` is Nullable so a record without a reported date still lands.
create table if not exists raw.ownership_record (
  symbol String,
  kind LowCardinality(String),
  holder Nullable(String),
  relationship Nullable(String),
  date Nullable(String),
  transaction_type Nullable(String),
  shares Nullable(Float64) codec(ZSTD),
  value Nullable(Float64) codec(ZSTD),
  percentage Nullable(Float64) codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
order by (symbol, kind)
settings non_replicated_deduplication_window = 1000;

-- raw.etf_holding mirrors tdw_domain::EtfHolding: one ETF constituent from SEC
-- N-PORT (NPORT-P). ORDER BY (fund_symbol, holding_name) keeps a fund's
-- portfolio contiguous. `report_date` is a Nullable String (filings report a
-- period-of-report date that may be absent in a partial parse).
create table if not exists raw.etf_holding (
  fund_symbol String,
  cik Nullable(String),
  report_date Nullable(String),
  holding_name String,
  cusip Nullable(String),
  isin Nullable(String),
  balance Nullable(Float64) codec(ZSTD),
  value_usd Nullable(Float64) codec(ZSTD),
  weight_pct Nullable(Float64) codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
order by (fund_symbol, holding_name)
settings non_replicated_deduplication_window = 1000;

-- raw.treasury_auction mirrors tdw_domain::TreasuryAuction: one auctioned US
-- Treasury security from FiscalData. ORDER BY (cusip, auction_date) keeps a
-- security's auction history contiguous. Rate/amount columns are Nullable since
-- FiscalData reports a different subset per security type.
create table if not exists raw.treasury_auction (
  cusip String,
  security_type LowCardinality(Nullable(String)),
  security_term LowCardinality(Nullable(String)),
  auction_date String,
  issue_date Nullable(String),
  maturity_date Nullable(String),
  high_yield Nullable(Float64) codec(ZSTD),
  interest_rate Nullable(Float64) codec(ZSTD),
  offering_amount Nullable(Float64) codec(ZSTD),
  bid_to_cover_ratio Nullable(Float64) codec(ZSTD),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
order by (cusip, auction_date)
settings non_replicated_deduplication_window = 1000;

-- raw.treasury_price mirrors tdw_domain::TreasuryPrice: one (cusip, date)
-- reference price from FiscalData. ORDER BY (cusip, date) keeps a security's
-- price history contiguous.
create table if not exists raw.treasury_price (
  cusip String,
  security_type LowCardinality(Nullable(String)),
  date String,
  price Nullable(Float64) codec(ZSTD),
  coupon_rate Nullable(Float64) codec(ZSTD),
  maturity_date Nullable(String),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
order by (cusip, date)
settings non_replicated_deduplication_window = 1000;

-- raw.fomc_document mirrors tdw_domain::FomcDocument: one FOMC document index
-- entry from the Federal Reserve. No event date guaranteed; ORDER BY
-- (doc_type, date) groups by document type. `date`/`url`/`title` are Nullable.
create table if not exists raw.fomc_document (
  doc_type LowCardinality(String),
  date Nullable(String),
  title Nullable(String),
  url Nullable(String),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
order by (doc_type)
settings non_replicated_deduplication_window = 1000;
