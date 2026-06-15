# tdw-bot Readiness Worksheet

Generated during the openbb-ecosystem-p1 **G007** landing (the chat-bot surface —
the chat-bot-equivalent, a differentiator). A Discord + Telegram chat bot over the
warehouse REST/catalog and charts: a chat message becomes a parsed command, the
core router maps it to a data fetch through an abstract seam, and the result is
rendered to a platform-agnostic `BotResponse` (text / table / chart).

## Evidence Snapshot

- Manifest: `crates/tdw-bot/Cargo.toml`.
- Targets: lib, plus two feature-gated bins (`tdw-bot-discord`,
  `tdw-bot-telegram`) declared with `required-features` so a default
  `cargo build --workspace` never compiles them.
- Local deps: `tdw-charting` (Plotly figure-spec builder) and `tdw-domain`
  (`MarketDataBar` / `LinePoint` series shapes). Plus `serde` / `serde_json`.
- Optional deps (all behind NON-DEFAULT features): `plotters` + `png` (the
  `charts` PNG rasterizer), `serenity` (the `discord` transport), `teloxide`
  (the `telegram` transport), and `tokio` + `reqwest` (the async runtime + the
  REST data client the transports wire in).
- Reverse deps: none (the bot is a leaf surface; a host deploys a transport bin).
- Features: `default` is empty (the pure-Rust, offline core); `charts`
  (`dep:plotters`, `dep:png`); `discord` (`dep:serenity`, `dep:tokio`,
  `dep:reqwest`); `telegram` (`dep:teloxide`, `dep:tokio`, `dep:reqwest`).
- Tests: command-parser unit tests (valid `/quote` / `/news` / `/chart` /
  `/help`, whitespace tolerance, missing-ticker, malformed-ticker, non-command,
  unknown-verb); router-dispatch tests over a fake data seam (quote→table,
  news→list, chart→figure, help, unknown-command fallback, missing-ticker, and a
  fetch-error rendering as text not a panic); plus, under `--features charts`, a
  test that a PNG is produced from a sample series (verified by the PNG magic
  bytes). All default-feature tests run fully offline.
- Docs/examples: this worksheet plus module-level docs on every public item.

## Thin-client boundary

The platform's business logic lives in ONE framework-free place — the core
(`command` parser, `data::DataSource` seam, `router::route`, `response`
`BotResponse`, `chart` payload builder) — and is exercised offline with an
injected fake. The transport adapters (`adapters::discord`, `adapters::telegram`,
compiled only under their feature) do exactly three things: receive a message,
call `router::route`, and send the formatted reply. The production data seam,
`adapters::rest::RestDataSource`, is a thin blocking client over the warehouse
`GET /api/v1/{route}` surface that maps `/quote` → `equity/price/quote`, `/news`
→ `news/company`, and `/chart` → `equity/price/historical`. No business logic
lives in an adapter.

## Chart-rendering decision

`tdw-charting` emits a Plotly figure JSON *spec* (so a `plotly.js` client renders
it with zero native dependency), and rasterizing that JSON in pure Rust is not
feasible. So `/chart` always attaches the figure-JSON spec plus a one-line
caption summary; and under the NON-DEFAULT `charts` feature it additionally
rasterizes a simple line PNG straight from the close series with the pure-Rust
`plotters` bitmap backend (no native/system graphics dependency) encoded by the
pure-Rust `png` crate. The raster draws only the gridless line — no text, since a
font backend is deliberately not enabled to keep the build pure-Rust.

## CI-safety (feature gating)

Mirrors G005's `gui` gate: the bot frameworks (`serenity`, `teloxide`) are heavy
async deps and are OPTIONAL behind NON-DEFAULT features, so
`cargo build/clippy/test --workspace` compiles only the pure-Rust, framework-free,
fully-offline core with no network or async runtime. The transports select
`rustls` TLS backends (no openssl/native TLS), so the gated build stays C-free;
both `serenity` and `teloxide` were verified to compile and lint on the Windows
runner under `--features discord,telegram`.

## Clean-room

Built only against the PUBLIC `serenity` / `teloxide` framework contract and the
public `plotly.js` figure shape (already produced by `tdw-charting`). No reference
implementation was consulted; no provider source; this crate does not change the
endpoint catalog.

## Verdict

Ready with follow-ups. The chat-bot core (parser + router + `BotResponse` +
formatting + chart payload) is complete with unit tests and is pure-Rust/offline;
the Discord/Telegram transport adapters and the `charts` PNG path are
feature-gated and compile-clean. Out of scope for G007 and left as a later append:
slash-command registration (Discord application commands / Telegram bot menu)
instead of `/`-prefixed plain text, richer multi-arg commands (date ranges,
intervals), posting the chart as a native image/figure attachment rather than a
text summary in the adapters, and an integration test that drives a transport
against a stub HTTP server.
