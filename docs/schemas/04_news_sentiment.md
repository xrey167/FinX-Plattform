# Schema 04: news_sentiment

Canonical Rust struct: `NewsSentiment`.

| Field | Type | Required | Notes |
| --- | --- | --- | --- |
| id | string | yes | Stable news item identifier. |
| headline | string | yes | News headline. |
| body | string | yes | Body text or extracted summary. |
| published_at | string | yes | Publication timestamp. |
| symbols | string[] | yes | Referenced instruments. |
| sentiment_score | number | yes | Range -1.0 to 1.0. |
| source | string | yes | News or sentiment provider. |
