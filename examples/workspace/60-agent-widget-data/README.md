# 60 — The two-request widget-data round trip

When a question needs data from a dashboard widget, the copilot cannot fetch it
itself — it asks the **frontend** to. This is the stateless two-request pattern,
scripted here end to end against the offline agent bridge.

## What it teaches

- **Leg 1.** The copilot sees a primary widget but no data yet, so it emits a
  `get_widget_data` event naming the widget and **closes the stream**. No answer
  is streamed.
- **Leg 2.** The frontend fetches that widget's data and re-POSTs the same
  conversation with the rows folded in as a `tool` message. The copilot now
  streams the grounded answer (the folded rows reach its context) and closes
  with `citations`.

## Run

```sh
cargo run -p tdw-workspace-examples --bin ws-60-agent-widget-data --target-dir target
```

It drives both legs against one booted server and prints the SSE event order for
each, then shows the folded rows reaching the leg-2 answer.

## Next

Example 70 returns richer artifacts — tables and charts.
