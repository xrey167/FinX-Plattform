# 20 — Catalog-derived data backend

The real Workspace data backend: boot the in-memory daemon and serve the
`widgets.json` that is **derived** from the endpoint catalog — one widget per
`Fetch` route (60+ of them) — plus the live `/widget-data/...` endpoint backed by
the policy-guarded fetch path.

## What it teaches

- You do not hand-write `widgets.json` in production: the daemon derives it from
  the catalog, so widgets stay in lockstep with the routes.
- `/widget-data/equity/price/historical` runs the full fetch path and is still
  offline — it resolves the always-registered `fileset` fixture, so no provider
  key is needed.

## Run

```sh
cargo run -p tdw-workspace-examples --bin ws-20-backend-derived --target-dir target
```

It boots the workspace surface on an ephemeral port, reports how many widgets the
catalog derived, prints the `equity_price_historical` widget, and fetches AAPL
history offline.

## Next

Example 30 layers `apps.json` (saved dashboards) on top of these widgets.
