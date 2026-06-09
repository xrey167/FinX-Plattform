# tdw-provider-velodata — Architecture

## Module map

| Module | Gate | Responsibility |
| ------ | ---- | -------------- |
| `lib.rs` | always | Query types, data models (`FundingRate`, `AggregatedLiquidation`, `AggregatedOi`), error enum (`VelodataProviderError`), validation, and the `PROVIDER_ID` / `BASE_URL` / `API_KEY_ENV` / `API_KEY_HEADER` constants. |
| `http_fetcher.rs` | `feature = "http"` | The three `Fetcher` implementations and the shared `read_api_key` / `get_bytes` helpers. |

## Traits implemented

All three fetchers implement `tdw_core::Fetcher<Q, D>`:

| Type | `Q` | `D` | `PROVIDER` / `ENDPOINT` |
| ---- | --- | --- | ----------------------- |
| `VelodataHttpFundingFetcher` | `VelodataFundingQuery` | `FundingRate` | `velodata` / `funding_rates` |
| `VelodataHttpLiquidationsFetcher` | `VelodataLiquidationsQuery` | `AggregatedLiquidation` | `velodata` / `liquidations_aggregated` |
| `VelodataHttpOiFetcher` | `VelodataOiQuery` | `AggregatedOi` | `velodata` / `oi_aggregated` |

## Data flow

```
transform_query (Value -> Q)  ->  extract_data (Q -> Bytes, async IO)
                              ->  transform_data (Bytes -> Vec<D>, pure)
```

1. `transform_query` parses and validates the JSON `Value` (exchange/symbol
   character checks; path-injection is rejected).
2. `extract_data` reads `TDW_VELODATA_API_KEY`, sends the `X-API-KEY` header,
   and issues the GET with `interval`/`limit` query parameters.
3. `transform_data` deserialises directly into the public row models. The
   Velo wire fields `fundingRate` / `fundingRateAnnualized` (and the
   liquidation/OI equivalents) are mapped via `serde(rename = …)`, so the
   public structs are the deserialisation targets.

## Offline / cassette design

`transform_data` is pure over `Bytes` — in fact it is a straight
`serde_json::from_slice` into the public models — so the parsing path is tested
and demonstrated offline against inline Velo JSON cassettes. `with_base_url(..)`
retargets `extract_data` at a local stub in integration tests. Live network
access requires the `http` feature and a real key.

## Clean-room invariants

- `#![forbid(unsafe_code)]` via workspace lints.
- No captured Velo responses are committed; only synthetic fixtures shaped like
  the documented schema appear in tests and the example.
- `reqwest` / `tokio` are optional, gated behind `http`; the default build is
  offline and deterministic.
- The crate talks only to documented Velo REST endpoints over the `X-API-KEY`
  header — no scraping or private APIs.
