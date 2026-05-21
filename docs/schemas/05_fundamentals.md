# Schema 05: fundamentals

Canonical Rust struct: `FundamentalMetric`.

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| symbol | string | yes | Instrument symbol. |
| fiscal_period | string | yes | Fiscal period key. |
| metric | string | yes | Metric name. |
| value | number | yes | Metric value. |
| currency | string | no | Currency when the metric is monetary. |
| reported_at | string | yes | Report timestamp. |
