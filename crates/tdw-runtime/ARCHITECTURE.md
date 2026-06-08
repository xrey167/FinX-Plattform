# tdw-runtime architecture

`tdw-runtime` is the provider execution seam: it pairs a `ProviderRegistry` with
`Credentials` and runs a `Fetcher`, exposing both a terminal fetch and a
progress-wrapped streaming variant.

## Module map

A single `src/lib.rs`.

## Key types

### `CommandRunner`

```text
CommandRunner {
    registry: ProviderRegistry,   // which providers/endpoints exist
    creds:    Credentials,        // passed to each fetch
}
```

Construction and configuration:

- `CommandRunner::new(registry)` / `CommandRunner::default()`.
- `with_credentials(creds)` — builder that sets the credentials handed to
  `Fetcher::extract_data`.
- `register_provider(RegistryEntry)` — add an entry; `registered_providers()`
  returns the current entries.

Execution (generic over `F: Fetcher<Q, D>`, `Q: QueryParams`, `D: DataModel`):

- `run(&fetcher, params: Value) -> Result<OBBject<D>>` — async terminal fetch:
  delegates to `fetcher.fetch(params, &self.creds)`.
- `run_streaming(&fetcher, params) -> Result<ProgressStream<D>>` — runs the
  terminal fetch, then yields a ready stream of:
  `Progress { stage: "fetch", fraction: 0.0 }`,
  `Progress { stage: "fetch", fraction: 1.0 }`,
  `Done(OBBject)`.

### `ReadyProgressStream<T>` (private)

A `futures_core::Stream` over a `VecDeque` of pre-computed
`Result<ProgressOrResult<T>>` items. `poll_next` pops the front item and returns
`Poll::Ready` immediately — there is no real async work, so a terminal fetch is
adapted into the daemon's streaming shape without a live subscription.

## Runtime flow

```text
caller ── params: Value ──▶ CommandRunner::run
                              │
              Fetcher::fetch (transform_query → extract_data → transform_data)
                              │
                              ▼
                       OBBject<D>            (terminal)

caller ──▶ CommandRunner::run_streaming
                              │
                       run(...)  ──▶ OBBject<D>
                              │
              wrap in ReadyProgressStream: [Progress 0.0, Progress 1.0, Done]
                              ▼
                       ProgressStream<D>
```

## Security posture

No I/O or auth of its own. `Credentials` flow through to the `Fetcher`; the
runner never logs or persists them. Trust/authorization is enforced upstream in
`tdw-service-api`'s policy guard before a `CommandRunner` is ever invoked.

## Integration points

- `tdw-core` — provides `Fetcher`, `ProviderRegistry`, `RegistryEntry`,
  `Credentials`, `OBBject`, `ProgressStream`, `ProgressOrResult`.
- `tdw-service-api` — constructs a `CommandRunner` from `AppState.registry` for
  `dispatch_ingest` and `fetch_equity_historical`.
- `tdw-provider-*` — supply the concrete `Fetcher` implementations.
