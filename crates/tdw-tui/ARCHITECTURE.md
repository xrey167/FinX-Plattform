# tdw-tui architecture

`tdw-tui` is a one-way formatter: `tdw_protocol::EventMsg` → `ratatui::text::Line`.
It holds no state and runs no loop; a TUI host owns the terminal and calls these
functions per frame.

## Module map

A single `src/lib.rs`.

## Key functions

- `event_lines(&[EventMsg]) -> Vec<Line<'static>>` — map a batch of events.
- `event_line(&EventMsg) -> Line<'static>` — map one event. Each variant renders
  a short label plus its sanitized dynamic fields:
  - `Started` → `"started"`
  - `Progress` → `"progress <stage> <fraction:.2>"`
  - `ApprovalRequested` → `"approval <action>"`
  - `ToolCallRequested` → `"tool requested <tool_name>"`
  - `ToolCallCompleted` → `"tool completed <call_id>"`
  - `OutputChunk` → `"<stream:?> <bytes>"`
  - `DomainEvent` → `"domain <event_type>"`
  - `Completed` → `"completed <summary?>"`
  - `Failed` → `"failed <error>"`
  - `Cancelled` → `"cancelled <reason?>"`
- `sanitize_event_text(&str) -> String` — replace every control character with a
  space and truncate to `MAX_EVENT_TEXT_LEN` (160) chars, appending `"..."` when
  truncated.

## Runtime flow

```text
daemon EventMsg(s)  ──▶  event_lines / event_line
                              │  (per dynamic field)
                              ▼
                      sanitize_event_text  (strip control chars, cap length)
                              ▼
                      ratatui Line<'static>  ──▶  host renders the frame
```

## Security posture

The sanitizer is the security-relevant surface: event text can originate from
provider output, tool results, or error strings, so every dynamic field is passed
through `sanitize_event_text` before it reaches the terminal. This neutralizes
ANSI/escape injection (control chars become spaces) and bounds line length so a
hostile payload cannot blow up the display. The crate performs no terminal I/O
itself, so it cannot leak or execute anything — it only produces inert `Line`s.

## Integration points

- `tdw-protocol` — the `EventMsg` / `OutputStream` source types.
- `ratatui` — the `Line` output type.
- `tdw-service-api::client_event_sample` — formats a run's events via
  `event_lines` as part of its deterministic evidence.
