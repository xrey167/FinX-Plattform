# 10 — Minimal data backend (hand-written)

The smallest possible OpenBB Workspace data backend: a hand-written
`widgets.json` describing **one** widget, plus the single data endpoint it points
at, served over a tiny standalone HTTP server. No daemon, no framework — just the
raw contract so you can see exactly what Workspace ingests.

## What it teaches

- A `widgets.json` is a JSON object keyed by widget id; each value declares the
  widget's `type`, its data `endpoint`, its editable `params`, and the `dataKey`
  under which rows arrive.
- A data endpoint returns `{ "<dataKey>": [ ...rows... ] }`.

## Run

```sh
cargo run -p tdw-workspace-examples --bin ws-10-backend-minimal --target-dir target
```

It boots the backend on an ephemeral loopback port, fetches `GET /widgets.json`
and `GET /widget-data/example/price`, and prints both documents.

## Next

Example 20 replaces this hand-work with the real, catalog-derived backend.
