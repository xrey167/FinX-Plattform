{{ config(materialized='view', tags=['domain:fundamentals']) }}

with pivoted as (
  select
    symbol,
    fiscal_period,
    max(case when metric = 'assets' then value end) as assets,
    max(case when metric = 'liabilities' then value end) as liabilities
  from {{ ref('silver_balance_sheet') }}
  group by symbol, fiscal_period
)

select
  symbol,
  fiscal_period,
  assets,
  liabilities,
  liabilities / nullif(assets, 0) as debt_to_assets_ratio,
  (assets - liabilities) / nullif(assets, 0) as equity_ratio
from pivoted
