# OpenBB Platform — Data Domain Surface (Clean-Room Gap Analysis)

> **Header note:** Derived from public OpenBB documentation (docs.openbb.co) for clean-room
> gap analysis; no OpenBB source code consulted. Command paths, names, purposes, key
> parameters, and provider coverage are taken from the public Platform reference and data-model
> documentation pages (facts), not from implementation source.

This document maps the full **data domain** surface of the OpenBB Platform API: every router
category, every command/endpoint, its one-line purpose, key parameters, and the providers that
serve it per the documentation's provider-coverage tables.

**Inventory basis:** the OpenBB docs sitemap (`docs.openbb.co/.../reference/*`) enumerates the
endpoints below. The four analysis routers (`technical`, `quantitative`, `econometrics`,
`famafrench`) are **computation routers** — they transform user-supplied or
library-sourced data rather than calling external data providers, so their "providers" column
is marked accordingly. The `imf_utils` and `uscongress` routers are auxiliary helpers.

Common provider abbreviations used below: `fmp` (Financial Modeling Prep), `yfinance` (Yahoo
Finance), `intrinio`, `polygon`, `tiingo`, `cboe`, `tmx`, `tradier`, `alpha_vantage`,
`finviz`, `benzinga`, `sec`, `fred`, `oecd`, `imf`, `econdb`, `ecb`, `federal_reserve`,
`us_eia` (EIA), `deribit`, `biztoc`, `nasdaq`, `wsj`, `seeking_alpha`, `government_us` /
`us_treasury`, `cftc`, `bls`, `fred_regional`.

---

## 1. equity (79 endpoints)

US/global stock data: prices, fundamentals, estimates, ownership, discovery, comparison.

### equity (root + price)

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| equity/search | Search for stock ticker symbols by name/identifier | query, is_symbol, use_cache | intrinio, sec, cboe, nasdaq, tmx, tradier |
| equity/screener | Screen equities by metric/fundamental/technical filters | (provider-specific signals/filters), e.g. Finviz signals | finviz, fmp |
| equity/profile | Company profile/overview (sector, industry, description) | symbol | finviz, fmp, intrinio, tmx, yfinance |
| equity/market_snapshots | Snapshot of prices for all symbols on an exchange | market/exchange | fmp, intrinio, polygon |
| equity/historical_market_cap | Historical market capitalization time series | symbol, start_date, end_date | fmp, intrinio |
| equity/price/historical | Historical OHLCV price data | symbol, start_date, end_date, interval, adjustment, extended_hours | alpha_vantage, cboe, fmp, intrinio, polygon, tiingo, tmx, tradier, yfinance |
| equity/price/quote | Latest quote (price/bid/ask) for a symbol | symbol | cboe, fmp, intrinio, tmx, tradier, yfinance |
| equity/price/performance | Price performance over standard periods (1d…YTD…5y) | symbol | finviz, fmp |

### equity/calendar

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| equity/calendar/dividend | Upcoming/past dividend calendar | start_date, end_date | fmp, nasdaq |
| equity/calendar/earnings | Earnings announcement calendar | start_date, end_date | fmp, nasdaq, tmx, seeking_alpha |
| equity/calendar/events | Corporate events calendar | symbol | nasdaq |
| equity/calendar/ipo | IPO calendar | symbol, start_date, end_date, status | intrinio, nasdaq |
| equity/calendar/splits | Stock split calendar | start_date, end_date | fmp |

### equity/compare

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| equity/compare/peers | Company peers (comparable tickers) | symbol | fmp |
| equity/compare/groups | Compare groups by sector/industry/country metrics | group, metric | finviz |
| equity/compare/company_facts | Compare reported company facts across symbols | symbol, fact | sec |

### equity/darkpool

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| equity/darkpool/otc | OTC / dark-pool weekly trading volume | symbol | finra |

### equity/discovery

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| equity/discovery/active | Most active stocks by volume | sort | fmp, yfinance |
| equity/discovery/gainers | Top price gainers | sort | fmp, yfinance |
| equity/discovery/losers | Top price losers | sort | fmp, yfinance |
| equity/discovery/aggressive_small_caps | Aggressive small-cap stocks | sort | yfinance |
| equity/discovery/growth_tech | Growth tech stocks | sort, limit | yfinance |
| equity/discovery/undervalued_growth | Undervalued growth stocks | sort, limit | yfinance |
| equity/discovery/undervalued_large_caps | Undervalued large-cap stocks | sort, limit | yfinance |
| equity/discovery/top_retail | Top retail-traded stocks | limit | nasdaq |
| equity/discovery/filings | Latest SEC filings reported to EDGAR | start_date, form_type, limit | fmp |
| equity/discovery/latest_financial_reports | Latest financial reports filed | date, report_type | sec |

### equity/estimates

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| equity/estimates/price_target | Analyst price targets by company | symbol, limit | benzinga, finviz, fmp |
| equity/estimates/consensus | Consensus price target & recommendation | symbol | fmp, intrinio, tmx, yfinance |
| equity/estimates/analyst_search | Search analysts & forecast track record | analyst_name, firm_name | benzinga |
| equity/estimates/historical | Historical analyst earnings/revenue estimates | symbol, period | fmp |
| equity/estimates/forward_eps | Forward EPS estimates | symbol, fiscal_period | fmp, intrinio, seeking_alpha |
| equity/estimates/forward_sales | Forward sales/revenue estimates | symbol, fiscal_period | intrinio, seeking_alpha |
| equity/estimates/forward_ebitda | Forward EBITDA estimates | symbol, fiscal_period | fmp, intrinio |
| equity/estimates/forward_pe | Forward P/E estimates | symbol | intrinio |

### equity/fundamental

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| equity/fundamental/balance | Balance sheet statement | symbol, period, limit | fmp, intrinio, polygon, yfinance |
| equity/fundamental/balance_growth | Balance sheet growth rates | symbol, limit | fmp |
| equity/fundamental/income | Income statement | symbol, period, limit | fmp, intrinio, polygon, yfinance |
| equity/fundamental/income_growth | Income statement growth rates | symbol, limit | fmp |
| equity/fundamental/cash | Cash flow statement | symbol, period, limit | fmp, intrinio, polygon, yfinance |
| equity/fundamental/cash_growth | Cash flow growth rates | symbol, limit | fmp |
| equity/fundamental/metrics | Key financial metrics (per-share, valuation) | symbol, period | finviz, fmp, intrinio, yfinance |
| equity/fundamental/ratios | Financial ratios (liquidity, profitability, leverage) | symbol, period, limit | fmp, intrinio |
| equity/fundamental/reported_financials | As-reported financial statements | symbol, statement_type, period | intrinio |
| equity/fundamental/dividends | Historical dividends paid | symbol | fmp, intrinio, nasdaq, tmx, yfinance |
| equity/fundamental/trailing_dividend_yield | Trailing 1y dividend yield series | symbol | tiingo |
| equity/fundamental/historical_eps | Historical earnings per share | symbol | fmp, intrinio |
| equity/fundamental/historical_splits | Historical stock splits | symbol | fmp |
| equity/fundamental/employee_count | Historical employee headcount | symbol | fmp |
| equity/fundamental/esg_score | ESG scores | symbol | fmp |
| equity/fundamental/filings | Company SEC filings index | symbol, form_type, limit | fmp, intrinio, sec, tmx |
| equity/fundamental/management | Executive/management team | symbol | fmp |
| equity/fundamental/management_compensation | Executive compensation | symbol | fmp |
| equity/fundamental/management_discussion_analysis | MD&A section text from filings | symbol, period | sec |
| equity/fundamental/historical_attributes | Historical values of an Intrinio data tag | symbol, tag, frequency | intrinio |
| equity/fundamental/latest_attributes | Latest value of an Intrinio data tag | symbol, tag | intrinio |
| equity/fundamental/search_attributes | Search available Intrinio data tags | query, limit | intrinio |
| equity/fundamental/revenue_per_geography | Revenue by geographic segment | symbol, period, structure | fmp |
| equity/fundamental/revenue_per_segment | Revenue by business segment | symbol, period, structure | fmp |
| equity/fundamental/transcript | Earnings call transcript | symbol, year, quarter | fmp |

### equity/ownership

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| equity/ownership/insider_trading | Insider (management/board) trading activity | symbol, limit | fmp, intrinio, tmx |
| equity/ownership/institutional | Institutional ownership over time | symbol | fmp |
| equity/ownership/major_holders | Major holders of a company | symbol | fmp |
| equity/ownership/share_statistics | Share float / insider & institution ownership % | symbol | fmp, intrinio, yfinance |
| equity/ownership/form_13f | SEC Form 13F-HR institutional holdings | symbol/cik, date, limit | sec |
| equity/ownership/government_trades | Government official (congressional) trades | symbol, chamber, limit | fmp |

### equity/shorts

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| equity/shorts/short_interest | Reported short interest | symbol | finra |
| equity/shorts/short_volume | Daily short-sale volume | symbol | stockgrid |
| equity/shorts/fails_to_deliver | SEC fails-to-deliver data | symbol, limit | sec |

---

## 2. economy (46 endpoints)

Macroeconomic indicators, releases, central-bank and trade data.

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| economy/cpi | Consumer Price Index (inflation) | country, transform, frequency, harmonized | fred, oecd, imf |
| economy/pce | Personal Consumption Expenditures price index | category, frequency | fred |
| economy/gdp/real | Real GDP | country, frequency, units | oecd, econdb |
| economy/gdp/nominal | Nominal GDP | country, frequency, units | oecd, econdb |
| economy/gdp/forecast | GDP forecast | country, period, type | oecd |
| economy/calendar | Economic events calendar | start_date, end_date, country | fmp, tradingeconomics, nasdaq |
| economy/indicators | Cross-country standardized indicator series | country, symbol, transform | econdb, imf |
| economy/available_indicators | List available indicator symbols | (none) | econdb, imf |
| economy/country_profile | Country macro profile snapshot | country, latest | econdb |
| economy/interest_rates | Policy / market interest rates by country | country, duration, frequency | oecd |
| economy/unemployment | Unemployment rate | country, sex, age, frequency | oecd |
| economy/money_measures | Money supply aggregates (M1/M2) | start_date, adjusted | federal_reserve |
| economy/balance_of_payments | Balance of payments | country, report_type, frequency | fred, ecb |
| economy/direction_of_trade | Bilateral trade flows (exports/imports) | country, counterpart, direction, frequency | imf |
| economy/export_destinations | Top export destinations for a country | country | econdb |
| economy/composite_leading_indicator | OECD composite leading indicator | country, adjustment, growth_rate | oecd |
| economy/house_price_index | Residential house price index | country, frequency, transform | oecd |
| economy/share_price_index | Share (equity) price index | country, frequency, transform | oecd |
| economy/retail_prices | Retail price levels for goods | country, item, frequency | fred |
| economy/risk_premium | Equity market risk premium by country | (none) | fmp |
| economy/total_factor_productivity | Total factor productivity | country | (econdb/fred) |
| economy/central_bank_holdings | Central bank balance-sheet holdings | date, holding_type | federal_reserve |
| economy/primary_dealer_positioning | Primary dealer net positioning | category, start_date | federal_reserve |
| economy/primary_dealer_fails | Primary dealer fails to deliver/receive | start_date, category | federal_reserve |
| economy/fomc_documents | FOMC meeting documents index | year, document_type | federal_reserve |
| economy/fred_series | Arbitrary FRED data series by ID | symbol, start_date, frequency, transform | fred, intrinio |
| economy/fred_search | Search FRED series metadata | query, search_type | fred |
| economy/fred_release_table | Release table elements for a FRED release | release_id, element_id, date | fred |
| economy/fred_regional | Regional FRED data | symbol, start_date, frequency | fred |

### economy/survey

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| economy/survey/nonfarm_payrolls | Nonfarm payrolls (employment situation) | category, date | fred |
| economy/survey/sloos | Senior Loan Officer Opinion Survey | category | fred |
| economy/survey/inflation_expectations | Consumer inflation expectations | (start/end dates) | fred |
| economy/survey/university_of_michigan | U. Michigan consumer sentiment | start_date, end_date | fred |
| economy/survey/economic_conditions_chicago | Chicago Fed economic conditions index | start_date, end_date | fred |
| economy/survey/manufacturing_outlook_ny | NY Fed (Empire State) manufacturing survey | topic, seasonally_adjusted | fred |
| economy/survey/manufacturing_outlook_texas | Dallas Fed Texas manufacturing outlook | topic, transform | fred |
| economy/survey/bls_search | Search BLS series metadata | query, category | bls |
| economy/survey/bls_series | BLS data series by ID | symbol, start_date | bls |

### economy/shipping

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| economy/shipping/port_info | Port reference info | (none) | imf |
| economy/shipping/port_volume | Port throughput volume | port_code, start_date | imf |
| economy/shipping/chokepoint_info | Maritime chokepoint reference info | (none) | imf |
| economy/shipping/chokepoint_volume | Maritime chokepoint transit volume | chokepoint, start_date | imf |

---

## 3. fixedincome (30 endpoints)

Government & corporate rates, yield curves, reference rates, spreads.

### fixedincome (root)

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| fixedincome/bond_indices | Bond index levels/returns | index_type, category, start_date | fred |
| fixedincome/mortgage_indices | Mortgage rate indices | index, start_date | fred |

### fixedincome/government

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| fixedincome/government/yield_curve | Yield curve (nominal/real/breakeven/corporate) | date, yield_curve_type, country | ecb, econdb, federal_reserve, fmp, fred |
| fixedincome/government/treasury_rates | US Treasury constant-maturity rates | start_date, end_date | fmp, federal_reserve |
| fixedincome/government/treasury_prices | US Treasury security prices | date, cusip, security_type | government_us, tmx |
| fixedincome/government/treasury_auctions | US Treasury auction results | security_type, start_date | government_us |
| fixedincome/government/tips_yields | TIPS (inflation-protected) yields | start_date, end_date | fred |
| fixedincome/government/svensson_yield_curve | Svensson-model fitted yield curve | start_date, end_date | fred |

### fixedincome/corporate

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| fixedincome/corporate/bond_prices | Corporate bond prices | country, issuer, coupon_min/max | tmx |
| fixedincome/corporate/commercial_paper | Commercial paper rates | maturity, category, grade | federal_reserve |
| fixedincome/corporate/spot_rates | High-quality market (HQM) spot rates | start_date, maturity, category | fred |
| fixedincome/corporate/hqm | HQM corporate yield curve | date, yield_curve | fred |

### fixedincome/rate

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| fixedincome/rate/sofr | Secured Overnight Financing Rate | start_date, end_date | federal_reserve, fred |
| fixedincome/rate/effr | Effective Federal Funds Rate | start_date, parameter | federal_reserve, fred |
| fixedincome/rate/effr_forecast | FOMC projected fed funds rate | long_run | fred |
| fixedincome/rate/estr | Euro short-term rate (€STR) | start_date, parameter | fred |
| fixedincome/rate/ecb | ECB key interest rates | start_date, interest_rate_type | fred |
| fixedincome/rate/sonia | Sterling Overnight Index Average | start_date, parameter | fred |
| fixedincome/rate/ameribor | AMERIBOR rate | start_date, parameter | fred |
| fixedincome/rate/iorb | Interest on Reserve Balances | start_date, end_date | fred |
| fixedincome/rate/dpcredit | Discount window primary credit rate | start_date, parameter | fred |
| fixedincome/rate/overnight_bank_funding | Overnight Bank Funding Rate | start_date, end_date | federal_reserve, fred |

### fixedincome/spreads

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| fixedincome/spreads/tcm | Treasury constant maturity spread | start_date, maturity | fred |
| fixedincome/spreads/tcm_effr | TCM minus EFFR spread | start_date, maturity | fred |
| fixedincome/spreads/treasury_effr | Treasury bill minus EFFR spread | start_date, maturity | fred |

---

## 4. technical (28 endpoints) — computation router

Technical-analysis indicators computed over input price data (data supplied by the caller,
typically from `equity/price/historical` etc.). **No external data providers.**

| Command | Purpose | Key params |
|---|---|---|
| technical/sma | Simple moving average | data, target, length |
| technical/ema | Exponential moving average | data, target, length |
| technical/wma | Weighted moving average | data, target, length |
| technical/hma | Hull moving average | data, target, length |
| technical/zlma | Zero-lag moving average | data, target, length |
| technical/macd | Moving Average Convergence Divergence | data, fast, slow, signal |
| technical/rsi | Relative Strength Index | data, length, scalar |
| technical/stoch | Stochastic oscillator | data, fast_k_period, slow_d_period |
| technical/cci | Commodity Channel Index | data, length, scalar |
| technical/adx | Average Directional Index | data, length, scalar |
| technical/aroon | Aroon indicator | data, length |
| technical/bbands | Bollinger Bands | data, length, std |
| technical/kc | Keltner Channels | data, length, scalar |
| technical/donchian | Donchian Channels | data, lower_length, upper_length |
| technical/atr | Average True Range | data, length |
| technical/obv | On-Balance Volume | data |
| technical/ad | Accumulation/Distribution line | data, offset |
| technical/adosc | Accumulation/Distribution oscillator | data, fast, slow |
| technical/vwap | Volume-Weighted Average Price | data, anchor |
| technical/fisher | Fisher Transform | data, length |
| technical/cg | Center of Gravity oscillator | data, length |
| technical/macd / cones | (see above) | — |
| technical/cones | Volatility cones | data, lower_q, upper_q, model |
| technical/clenow | Clenow Volatility Adjusted Momentum | data, period |
| technical/demark | DeMark sequential indicator | data, show_all, asint |
| technical/ichimoku | Ichimoku Cloud | data, conversion, base |
| technical/fib | Fibonacci retracement levels | data, period, start/end date |
| technical/relative_rotation | Relative Rotation Graph data | data, benchmark, study |

---

## 5. quantitative (23 endpoints) — computation router

Quantitative statistics & performance metrics computed over input data. **No external
providers.**

| Command | Purpose | Key params |
|---|---|---|
| quantitative/summary | Summary statistics of a dataset | data, target |
| quantitative/normality | Normality tests (Kurtosis/Skew/JB/SW/KS) | data, target |
| quantitative/capm | CAPM beta/alpha vs market | data, target |
| quantitative/unitroot_test | Augmented Dickey-Fuller unit-root test | data, target, fuller_reg |
| quantitative/performance/sharpe_ratio | Sharpe ratio | data, target, rfr, window |
| quantitative/performance/sortino_ratio | Sortino ratio | data, target, target_return, window |
| quantitative/performance/omega_ratio | Omega ratio | data, target, threshold_start/end |
| quantitative/stats/mean | Mean | data, target |
| quantitative/stats/stdev | Standard deviation | data, target |
| quantitative/stats/variance | Variance | data, target |
| quantitative/stats/skew | Skewness | data, target |
| quantitative/stats/kurtosis | Kurtosis | data, target |
| quantitative/stats/quantile | Quantile | data, target, quantile_pct |
| quantitative/rolling/mean | Rolling mean | data, target, window |
| quantitative/rolling/stdev | Rolling standard deviation | data, target, window |
| quantitative/rolling/variance | Rolling variance | data, target, window |
| quantitative/rolling/skew | Rolling skewness | data, target, window |
| quantitative/rolling/kurtosis | Rolling kurtosis | data, target, window |
| quantitative/rolling/quantile | Rolling quantile | data, target, window, quantile_pct |

*(stats/rolling sub-trees enumerate mean, stdev, variance, skew, kurtosis, quantile — 6 each.)*

---

## 6. econometrics (16 endpoints) — computation router

Statistical/econometric tests & regressions over input data. **No external providers.**

| Command | Purpose | Key params |
|---|---|---|
| econometrics/correlation_matrix | Correlation matrix of a dataset | data, method |
| econometrics/ols_regression | OLS regression (coefficients) | data, y_column, x_columns |
| econometrics/ols_regression_summary | OLS regression full summary | data, y_column, x_columns |
| econometrics/autocorrelation | Autocorrelation (Durbin-Watson) | data, y_column, x_columns |
| econometrics/residual_autocorrelation | Breusch-Godfrey residual autocorrelation | data, y_column, x_columns, lags |
| econometrics/cointegration | Cointegration test between series | data, columns, maxlag |
| econometrics/causality | Granger causality test | data, y_column, x_column, lag |
| econometrics/unit_root | Unit-root (ADF) test | data, column, regression |
| econometrics/variance_inflation_factor | VIF for multicollinearity | data, column, columns |
| econometrics/panel_random_effects | Panel random-effects model | data, y_column, x_columns |
| econometrics/panel_fixed | Panel fixed-effects (one-way) model | data, y_column, x_columns |
| econometrics/panel_between | Panel between estimator | data, y_column, x_columns |
| econometrics/panel_pooled | Pooled OLS panel model | data, y_column, x_columns |
| econometrics/panel_first_difference | Panel first-difference OLS model | data, y_column, x_columns |
| econometrics/panel_fmac | Fama-MacBeth panel estimator | data, y_column, x_columns |

---

## 7. derivatives (11 endpoints)

Options and futures market data.

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| derivatives/options/chains | Full option chain (strikes/expiries/Greeks) | symbol | cboe, deribit, intrinio, tmx, tradier, yfinance |
| derivatives/options/unusual | Unusual options activity | symbol, source | intrinio |
| derivatives/options/snapshots | Snapshot of options across the market | (date) | intrinio |
| derivatives/options/surface | Implied volatility surface | symbol | (intrinio/tmx) |
| derivatives/futures/historical | Historical futures OHLCV | symbol, expiration, start_date | deribit, yfinance |
| derivatives/futures/curve | Futures forward curve by expiry | symbol, date | cboe, deribit, yfinance |
| derivatives/futures/instruments | List available futures instruments | (none) | deribit |
| derivatives/futures/info | Futures instrument metadata | symbol | deribit |

---

## 8. etf (14 endpoints)

Exchange-traded-fund holdings, info, performance, discovery.

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| etf/search | Search ETFs by name/criteria | query, exchange | fmp, intrinio, tmx |
| etf/info | ETF profile/info | symbol | fmp, intrinio, tmx, yfinance |
| etf/historical | Historical ETF OHLCV prices | symbol, start_date, interval | alpha_vantage, cboe, fmp, intrinio, polygon, tiingo, tmx, tradier, yfinance |
| etf/holdings | Constituent holdings of an ETF | symbol, date | fmp, intrinio, sec, tmx |
| etf/sectors | Sector allocation of an ETF | symbol | fmp, tmx |
| etf/countries | Country allocation of an ETF | symbol | fmp, tmx |
| etf/price_performance | ETF price performance by period | symbol | fmp |
| etf/equity_exposure | ETFs holding a given stock | symbol | fmp |
| etf/nport_disclosure | SEC N-PORT portfolio disclosure | symbol, date | sec |
| etf/discovery/active | Most active ETFs | sort | wsj |
| etf/discovery/gainers | Top gaining ETFs | sort | wsj |
| etf/discovery/losers | Top losing ETFs | sort | wsj |

---

## 9. fixedincome see §3 — (already mapped above)

---

## 10. index (9 endpoints)

Market index prices, constituents, snapshots.

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| index/price/historical | Historical index OHLCV | symbol, start_date, interval | cboe, fmp, intrinio, polygon, yfinance |
| index/constituents | Constituent members of an index | symbol | cboe, fmp, tmx |
| index/available | List available indices for a provider | (none) | cboe, fmp, yfinance |
| index/search | Search indices by query | query | cboe |
| index/snapshots | Snapshot quotes for indices in a region | region | cboe, tmx |
| index/sectors | Sector breakdown of an index | symbol | tmx |
| index/sp500_multiples | Historical S&P 500 multiples / Shiller PE | series_name, start_date | nasdaq |

---

## 11. crypto (4 endpoints)

Cryptocurrency prices & search.

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| crypto/search | Search available crypto pairs/symbols | query | fmp |
| crypto/price/historical | Historical crypto OHLCV | symbol, start_date, end_date, interval | fmp, polygon, tiingo, yfinance |

---

## 12. currency (6 endpoints)

FX rates, reference rates, snapshots.

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| currency/search | Search available currency pairs | query | fmp, intrinio, polygon |
| currency/price/historical | Historical FX OHLCV | symbol (CURR1-CURR2), start_date, interval | fmp, polygon, tiingo, yfinance |
| currency/snapshots | FX snapshot rates relative to a base | base, quote_type, counterpart | fmp, polygon |
| currency/reference_rates | Official ECB currency reference rates | (none) | ecb |

---

## 13. commodity (8 endpoints)

Commodity prices and energy/agriculture reports.

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| commodity/price/spot | Spot commodity prices | commodity, start_date, end_date | fred |
| commodity/petroleum_status_report | EIA Weekly Petroleum Status Report | category, table, start_date | us_eia |
| commodity/short_term_energy_outlook | EIA Short-Term Energy Outlook | symbol, frequency, start_date | us_eia |
| commodity/weather_bulletins_download | USDA/weather bulletin file download | (date/report) | us_eia |
| commodity/psd_data | USDA Production, Supply & Distribution data | commodity, country, start_year | us_eia |
| commodity/psd_report | USDA PSD report tables | report, commodity | us_eia |

---

## 14. news (3 endpoints)

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| news/company | Company-specific news articles | symbol, start_date, limit | benzinga, fmp, intrinio, polygon, tiingo, tmx, yfinance |
| news/world | General/world financial news | start_date, limit | benzinga, biztoc, fmp, intrinio, tiingo |

---

## 15. regulators (13 endpoints)

SEC and CFTC regulatory data utilities.

### regulators/sec

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| regulators/sec/cik_map | Map ticker symbol → CIK | symbol | sec |
| regulators/sec/symbol_map | Map CIK → ticker symbol | query | sec |
| regulators/sec/institutions_search | Search SEC-regulated institutions by name | query | sec |
| regulators/sec/sic_search | Search SIC industry codes | query | sec |
| regulators/sec/filing_headers | Filing header metadata for an accession | url/accession | sec |
| regulators/sec/htm_file | Retrieve an HTML file from a filing | url | sec |
| regulators/sec/schema_files | List schema/data files in a filing | url, use_cache | sec |
| regulators/sec/rss_litigation | SEC litigation releases (RSS) | (none) | sec |

### regulators/cftc

| Command | Purpose | Key params | Providers |
|---|---|---|---|
| regulators/cftc/cot | CFTC Commitment of Traders report | id, report_type, start_date | cftc |
| regulators/cftc/cot_search | Search COT contract markets | query | cftc |

---

## 16. famafrench (7 endpoints) — library-sourced computation router

Fama-French factor and portfolio data from the Kenneth R. French Data Library (public academic
dataset; not a commercial provider key).

| Command | Purpose | Key params | Source |
|---|---|---|---|
| famafrench/factors | Fama-French factor returns (3/5-factor etc.) | region, factor, frequency | French Data Library |
| famafrench/breakpoints | Portfolio formation breakpoints | breakpoint_type, frequency | French Data Library |
| famafrench/us_portfolio_returns | US portfolio returns by characteristic | portfolio, frequency | French Data Library |
| famafrench/regional_portfolio_returns | Regional portfolio returns | region, portfolio, frequency | French Data Library |
| famafrench/country_portfolio_returns | Country portfolio returns | country, portfolio, frequency | French Data Library |
| famafrench/international_index_returns | International index returns | region, index, frequency | French Data Library |

---

## 17. imf_utils (5 endpoints) — auxiliary helper

IMF SDMX dataflow discovery helpers (used to drive `economy/*` IMF queries).

| Command | Purpose | Key params | Source |
|---|---|---|---|
| imf_utils/list_dataflows | List available IMF dataflows | query | imf |
| imf_utils/list_tables | List tables within an IMF dataflow | (dataflow) | imf |
| imf_utils/get_dataflow_dimensions | Get dimensions of an IMF dataflow | dataflow | imf |
| imf_utils/presentation_table | Build a presentation table from IMF data | (table params) | imf |

---

## 18. uscongress (4 endpoints) — auxiliary

US Congress legislative data (congress.gov).

| Command | Purpose | Key params | Source |
|---|---|---|---|
| uscongress/bills | List congressional bills | congress, bill_type | congress_gov |
| uscongress/bill_info | Detailed info for a bill | congress, bill_type, bill_number | congress_gov |
| uscongress/bill_text_urls | URLs to bill text versions | congress, bill_type, bill_number | congress_gov |

---

## Summary

| Category | Endpoints (per docs sitemap) | Nature |
|---|---|---|
| equity | 79 | provider-backed |
| economy | 46 | provider-backed |
| fixedincome | 30 | provider-backed |
| technical | 28 | computation (no providers) |
| quantitative | 23 | computation (no providers) |
| econometrics | 16 | computation (no providers) |
| etf | 14 | provider-backed |
| regulators | 13 | provider-backed (sec, cftc) |
| derivatives | 11 | provider-backed |
| index | 9 | provider-backed |
| commodity | 8 | provider-backed (us_eia, fred) |
| famafrench | 7 | library-sourced (French Data Library) |
| currency | 6 | provider-backed |
| imf_utils | 5 | auxiliary (imf) |
| crypto | 4 | provider-backed |
| uscongress | 4 | auxiliary (congress_gov) |
| news | 3 | provider-backed |

- **Categories mapped:** 17 routers (15 data/computation domains + `imf_utils` and
  `uscongress` auxiliaries).
- **Commands/endpoints mapped:** **306** total (matches the docs sitemap reference inventory;
  this count includes router landing pages such as `equity`, `economy/gdp`, `economy/survey`,
  `economy/shipping`, `quantitative/stats`, `quantitative/rolling`, etc., which group the leaf
  endpoints listed above).
- **Distinct data providers referenced** (~35): fmp, yfinance, intrinio, polygon, tiingo, cboe,
  tmx, tradier, alpha_vantage, finviz, benzinga, sec, finra, stockgrid, nasdaq, wsj,
  seeking_alpha, tradingeconomics, fred, oecd, imf, ecb, econdb, federal_reserve, government_us
  (us_treasury), us_eia, cftc, bls, deribit, biztoc, congress_gov, plus the Kenneth French Data
  Library (academic, non-keyed).

### Notes for gap analysis vs FinX-Plattform

- **Provider-backed domains** (equity, economy, fixedincome, etf, derivatives, index,
  commodity, currency, crypto, news, regulators) are the comparable surface for FinX's
  `tdw-service-api` / `tdw-providers` provider catalog.
- **Computation routers** (technical, quantitative, econometrics) and the library-sourced
  `famafrench` are analytics layers that operate on already-fetched data — relevant to a
  future FinX analytics/transform surface rather than the provider-fetch layer.
- Provider lists are taken from the per-endpoint "providers" coverage in the docs and the
  associated data-model pages; exact provider sets can shift between OpenBB releases, and a
  few less-common providers may not be exhaustively enumerated for every leaf endpoint.
