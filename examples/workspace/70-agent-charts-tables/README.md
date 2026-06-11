# 70 — Table + chart artifacts

A copilot can return more than text: it can stream structured artifacts the
Workspace renders inline — a `table` of rows or a `chart`. This example takes a
small set of OHLCV bars (the kind a widget-data fetch returns) and emits both,
and also shows the same bars feeding the shared charting builder to produce a
full Plotly figure-spec.

## What it teaches

- The `table` SSE artifact: a named set of homogeneous row records.
- The `chart` SSE artifact: a `type` (line / bar / scatter), the `data` rows, and
  the `x_key` / `y_keys` that map columns to axes.
- Reuse: the same bars feed `tdw_charting::candlestick` to build the richer
  Plotly figure-spec that the REST / SDK surface exposes under `extra.chart`.

## Run

```sh
cargo run -p tdw-workspace-examples --bin ws-70-agent-charts-tables --target-dir target
```

It prints the SSE artifact transcript and the Plotly candlestick figure-spec.

## Next

Example 80 leaves Rust behind and drives the REST surface from Python.
