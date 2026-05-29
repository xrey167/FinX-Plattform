-- Corporate actions (splits / dividends / spinoffs) and a split-adjusted daily
-- close view. raw.corporate_action is the long-term system of record for
-- per-symbol corporate events; analytics.ohlc_1d_split_adjusted_v derives a
-- back-adjusted close so a historical price series is continuous across splits.
--
-- Typing / codec / dedup rationale matches the other bronze tables
-- (see 20260528_0002 / 0013): symbol plain String (high-cardinality leading
-- key); LowCardinality for the handful-of-values columns (action_type /
-- currency); ingested_at default now64(3) codec(DoubleDelta, ZSTD). The
-- non-idempotent ingested_at DEFAULT defeats content-hash dedup, so the ingest
-- path supplies an explicit insert_deduplication_token per batch; the
-- non_replicated_deduplication_window below lets that token take effect on this
-- non-replicated MergeTree table (swap to ReplicatedMergeTree in production).
--
-- NO TTL: unlike tick/trade/quote, corporate-action history is retained
-- long-term -- a split from years ago is still required to back-adjust an old
-- price series, so these rows never expire.
--
-- action_type is one of {'split','dividend','spinoff'}. ex_date is the
-- ex-effective date (a plain Date, the granularity at which corporate actions
-- apply to daily bars). split_ratio is the share multiplier for a split (e.g.
-- 4.0 for a 4-for-1 split); it defaults to 1.0 so dividend/spinoff rows are
-- split-neutral. cash_amount/currency carry the dividend cash leg.
create table if not exists raw.corporate_action (
  symbol String,
  ex_date Date,
  action_type LowCardinality(String),
  split_ratio Float64 default 1.0,
  cash_amount Float64 default 0.0,
  currency LowCardinality(String) default '',
  announced_at Date default toDate('1970-01-01'),
  ingested_at DateTime64(3, 'UTC') default now64(3) codec(DoubleDelta, ZSTD)
) engine = MergeTree
partition by toYYYYMM(ex_date)
order by (symbol, ex_date)
settings non_replicated_deduplication_window = 1000;

-- Split-adjusted daily close. For each daily bar, the back-adjusted close is the
-- raw close divided by the product of every split_ratio whose ex_date is STRICTLY
-- AFTER the bar's date -- i.e. all splits that occur in the bar's future. A price
-- printed before a 4-for-1 split is divided by 4 so it lines up with post-split
-- prices.
--
-- Implementation: a per-symbol stepped "future split factor" is built from the
-- split rows, then bars are ASOF LEFT JOIN-ed to it.
--
--   step 1 (future_split_factor): order each symbol's splits by ex_date
--           DESCENDING and take the running cumulative product of split_ratio via
--           exp(sum(log(split_ratio)) OVER (... ORDER BY ex_date DESC ...)). At a
--           given split row this yields the product of that split's ratio and all
--           later splits' ratios -- i.e. the factor that applies to any bar dated
--           on or before that split's ex_date.
--   step 2 (ASOF LEFT JOIN): for a bar at date d, find the split row with the
--           smallest ex_date such that ex_date > d (the next split in the future)
--           and use its cumulative factor. ASOF matches the closest row on an
--           inequality; bars with no future split fall through the LEFT JOIN and
--           use factor 1.0 (ifNull).
--
-- CERTAINTY:
--   * Window function avg/sum ... OVER (PARTITION BY ... ORDER BY ... DESC):
--     CERTAIN -- standard SQL window function, supported by ClickHouse.
--   * exp(sum(log(x)) OVER (...)) cumulative product: CERTAIN -- exp/log are
--     standard ClickHouse math functions; this is the well-established idiom for
--     a windowed product (ClickHouse has no product-aggregate).
--   * ASOF LEFT JOIN with a strict `>` inequality on the last key column:
--     CERTAIN of the general form (ASOF LEFT JOIN ... ON equi-keys AND
--     left.col < right.col), but the EXACT placement/strictness of the inequality
--     for a "next future split" (we need bar_date < split.ex_date, i.e. the
--     SMALLEST ex_date greater than bar_date) is the part most easily gotten
--     wrong on a store with no live validation. ClickHouse ASOF by default takes
--     the closest match where the right value is <= the left value on the ASOF
--     column, which is the OPPOSITE direction (it looks backward, not forward).
--     Rewriting "nearest future split" into that backward-looking primitive
--     correctly is non-trivial and unverifiable here. Per the correctness rule,
--     the joined view is therefore left as a TODO skeleton rather than guessed.
--
-- Both views below (split_factor_v and the ASOF-joined ohlc_1d_split_adjusted_v)
-- are now VALIDATED against live ClickHouse 26.6 — see the per-view notes for the
-- two corrections live testing surfaced (exp-outside-subquery, and the
-- if(fac>0,..) divisor for ASOF's default-0 no-match fill).

-- Per-symbol stepped future-split factor (CERTAIN primitives only):
-- factor(ex_date) = product of split_ratio for this split and all later splits.
-- NOTE (validated on live ClickHouse 26.6): the windowed cumulative product must
-- compute the window aggregate `sum(log(split_ratio)) OVER (...)` in a SUBQUERY
-- and apply `exp(...)` in the OUTER select. Writing `exp(sum(log(x)) OVER (...))`
-- inline fails — ClickHouse parses `exp(... OVER ...)` as a window aggregate named
-- `exp` and errors with "Aggregate function with name 'exp' does not exist".
create view if not exists analytics.split_factor_v
as select
  symbol,
  ex_date,
  exp(sum_log_ratio) as future_split_factor
from (
  select
    symbol,
    ex_date,
    sum(log(split_ratio)) OVER (
      PARTITION BY symbol ORDER BY ex_date DESC
      ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
    ) as sum_log_ratio
  from raw.corporate_action
  where action_type = 'split'
);

-- Split-adjusted daily close (VALIDATED on live ClickHouse 26.6). For each daily
-- bar, adjusted_close = close / (product of split_ratio for splits with ex_date
-- STRICTLY AFTER the bar's date). A price printed before a 4-for-1 split is
-- divided by 4 to line up with post-split prices; the bar ON the ex_date and
-- after is left unchanged (the `<` is strict).
--
-- ASOF LEFT JOIN picks, per bar, the split row with the smallest ex_date greater
-- than the bar date (`toDate(bars.window_start) < f.ex_date`) and uses its
-- cumulative future-split factor. Two live-confirmed subtleties:
--   * direction: ASOF with a `<` inequality on the right column matches the
--     NEAREST FUTURE split (smallest ex_date > bar date), which is what we want.
--   * no-match divisor: ASOF LEFT JOIN fills the right side with the column
--     DEFAULT (0.0 for Float64), NOT NULL — so a bar with no future split would
--     divide by 0 and yield +inf under ifNull/coalesce. We therefore guard with
--     `if(f.future_split_factor > 0, f.future_split_factor, 1.0)` (a real factor
--     is always >= 1, so 0 unambiguously means "no future split → factor 1").
create view if not exists analytics.ohlc_1d_split_adjusted_v
as select
  bars.symbol as symbol,
  bars.window_start as window_start,
  bars.close as close,
  bars.close / if(f.future_split_factor > 0, f.future_split_factor, 1.0) as adjusted_close
from analytics.ohlc_1d_consolidated_v as bars
asof left join analytics.split_factor_v as f
  on bars.symbol = f.symbol
 and toDate(bars.window_start) < f.ex_date;

-- FOLLOW-UP (documented, not implemented): dividend / total-return adjustment.
-- adjusted_close above accounts ONLY for splits. A total-return series additionally
-- reinvests cash dividends (cash_amount), typically via a (1 - dividend/close)
-- back-adjustment factor compounded across ex-dividend dates. That is a separate,
-- larger piece of work and is intentionally out of scope for this round.
