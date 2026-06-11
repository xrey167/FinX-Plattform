# 30 — Apps (saved dashboards)

`apps.json` is how a backend ships **saved dashboards**: named tabs, each a grid
of widget placements, plus starter prompts and the MCP servers the app's copilot
may use. This example serves the curated "FinX Market Overview" app the backend
ships and also builds a second, custom two-tab "Rates & Macro Board" app by hand
over the derived widget ids.

## What it teaches

- The `apps.json` shape: `tabs` (each with a `layout` of `{ i, x, y, w, h }`
  placements keyed by widget id), `prompts`, and `mcp_servers`.
- How to compose a custom app from the derived `widgets.json` keys (route with
  `/` replaced by `_`, e.g. `equity_price_historical`).
- Parameter sharing: a tab whose widgets all key on the same `symbol` forms a
  parameter group.

## Run

```sh
cargo run -p tdw-workspace-examples --bin ws-30-backend-app --target-dir target
```

It prints the curated `apps.json`, the custom app definition, and the merged
document a backend serving both would return.

## Next

Example 40 starts the copilot half of the surface.
