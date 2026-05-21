# Schema 06: strategy

Canonical Rust struct: `StrategySignal`.

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| strategy_id | string | yes | Strategy identifier. |
| symbol | string | yes | Instrument symbol. |
| side | enum | yes | Buy or sell. |
| score | number | yes | Strategy-specific confidence or ranking score. |
| target_weight | number | yes | Non-negative target portfolio weight. |
| generated_at | string | yes | Signal timestamp. |
| horizon | string | yes | Expected holding or evaluation horizon. |
