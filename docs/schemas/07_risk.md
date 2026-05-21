# Schema 07: risk

Canonical Rust struct: `RiskMetric`.

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| account_id | string | yes | Account scope for the metric. |
| metric | string | yes | Risk metric name. |
| value | number | yes | Observed value. |
| limit | number | no | Optional configured limit. |
| as_of | string | yes | Observation timestamp. |
| currency | string | no | Currency when monetary. |
