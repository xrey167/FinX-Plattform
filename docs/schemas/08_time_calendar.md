# Schema 08: time_calendar

Canonical Rust struct: `TradingCalendarEvent`.

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| calendar_id | string | yes | Calendar identifier. |
| venue | string | yes | Exchange or venue code. |
| session_date | string | yes | Session date. |
| open_ts | string | no | Optional open timestamp. |
| close_ts | string | no | Optional close timestamp. |
| is_trading_day | boolean | yes | Whether the venue is open for trading. |
| note | string | no | Optional calendar note. |
