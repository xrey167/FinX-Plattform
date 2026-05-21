# Schema 01: market_data

Canonical Rust structs: `MarketDataBar`, `EquityHistoricalData`.

## MarketDataBar Fields

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| symbol | string | yes | Instrument symbol. |
| venue | string | yes | Exchange or venue code. |
| granularity | enum | yes | Tick, minute, hour, day, week, or month. |
| ts | string | yes | Event timestamp. |
| open | number | yes | Non-negative open price. |
| high | number | yes | Non-negative high price. |
| low | number | yes | Non-negative low price. |
| close | number | yes | Non-negative close price. |
| volume | number | yes | Non-negative traded volume. |
| source | string | yes | Provider or fixture source. |

## EquityHistoricalData Fields

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| symbol | string | yes | Instrument symbol. |
| date | string | yes | Session date. |
| open | number | yes | Non-negative open price. |
| high | number | yes | Non-negative high price. |
| low | number | yes | Non-negative low price. |
| close | number | yes | Non-negative close price. |
| volume | integer | yes | Non-negative traded volume. |
