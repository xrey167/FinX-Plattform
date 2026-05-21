# Schema 10: reference_data

Canonical Rust structs: `ReferenceInstrument`, `Instrument`, `ResearchNote`.

## ReferenceInstrument Fields

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| symbol | string | yes | Instrument symbol. |
| name | string | yes | Display name. |
| venue | string | yes | Primary venue. |
| asset_class | enum | yes | Equity, ETF, future, option, forex, crypto, index, or fund. |
| currency | string | yes | Trading currency. |
| isin | string | no | Optional ISIN. |

## Instrument Fields

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| symbol | string | yes | Instrument symbol. |
| name | string | yes | Display name. |
| venue | string | yes | Primary venue. |

## ResearchNote Fields

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| id | string | yes | Stable note identifier. |
| title | string | yes | Note title. |
| body | string | yes | Note body text. |
| tags | string[] | yes | Search and retrieval tags. |
