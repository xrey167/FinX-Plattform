# Schema 11: costs_fees

Canonical Rust struct: `CostFeeEvent`.

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| event_id | string | yes | Stable cost or fee event identifier. |
| account_id | string | yes | Account scope for the fee. |
| order_id | string | no | Related order when available. |
| fee_type | string | yes | Commission, exchange, borrow, tax, spread, or other fee class. |
| amount | number | yes | Signed fee amount. |
| currency | string | yes | Fee currency. |
| charged_at | string | yes | Charge timestamp. |
