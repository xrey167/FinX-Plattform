# Schema 09: ops

Canonical Rust struct: `OperationalEvent`.

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| event_id | string | yes | Stable operational event identifier. |
| component | string | yes | Producing component. |
| severity | string | yes | Severity label. |
| observed_at | string | yes | Observation timestamp. |
| message | string | yes | Human-readable event message. |
| correlation_id | string | no | Optional correlation or trace id. |
