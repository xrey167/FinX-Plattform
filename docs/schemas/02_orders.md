# Schema 02: orders

Canonical Rust struct: `OrderEvent`.

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| order_id | string | yes | Stable order identifier. |
| account_id | string | yes | Account scope for the order. |
| symbol | string | yes | Instrument symbol. |
| side | enum | yes | Buy or sell. |
| status | enum | yes | New, partially filled, filled, cancelled, rejected, or expired. |
| quantity | number | yes | Non-negative requested quantity. |
| filled_quantity | number | yes | Non-negative filled quantity. |
| limit_price | number | no | Optional limit price. |
| event_ts | string | yes | Order-event timestamp. |
