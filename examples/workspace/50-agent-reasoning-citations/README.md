# 50 — Reasoning steps + citations

Beyond streaming text, a copilot narrates its work with `reasoning_step` events
and attributes its answer to the widgets it used with a closing `citations`
event. This example drives the **pure** copilot sequencer in-process over the
offline stub model on a conversation that already carries a folded widget-data
result, so the second-leg shape is produced with no server and no network.

## What it teaches

- `reasoning_step` events (`INFO` / `SUCCESS` / ...) narrate progress.
- The grounded-turn event order: an opening `reasoning_step`, streamed
  `message_chunk`s, a `SUCCESS` `reasoning_step`, then a `citations` event whose
  `source_widget_id` points back at the widget that grounded the answer.

## Run

```sh
cargo run -p tdw-workspace-examples --bin ws-50-agent-reasoning-citations --target-dir target
```

It prints the ordered events and the exact SSE wire transcript.

## Next

Example 60 shows how the widget data gets to the copilot in the first place —
the two-request round trip.
