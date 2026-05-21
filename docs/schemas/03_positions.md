# Schema 03: positions

Canonical Rust struct: `PositionSnapshot`.

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| account_id | string | yes | Account scope for the position. |
| symbol | string | yes | Instrument symbol. |
| as_of | string | yes | Snapshot timestamp. |
| quantity | number | yes | Signed position quantity. |
| average_price | number | yes | Average entry price. |
| market_value | number | yes | Current market value. |
| unrealized_pnl | number | yes | Unrealized profit and loss. |
| currency | string | yes | ISO-style currency code. |
