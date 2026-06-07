# tdw-cdc

Change-data-capture projection: turns durable outbox records into an ordered,
replayable change stream that consumers can tail from a known offset.

## Purpose

`tdw-cdc` is the read model over the transactional **outbox**. After events are
persisted by `tdw-outbox`, this crate projects them into a `CdcStream` of
`CdcRecord`s — one per outbox record — exposing the fields a downstream consumer
needs:

- `offset` — the outbox `sequence` (monotonic, gap-free ordering key);
- `event_id`, `event_type` — taken from the event envelope;
- `payload` — the raw `serde_json::Value` body.

Consumers tail the stream with `after(offset)`, getting every change strictly
after the last offset they processed — the foundation of at-least-once,
resumable delivery.

The crate is pure: `#![forbid(unsafe_code)]`, no I/O, no async.

## Feature flags

None.

## Dependencies

- `serde`, `serde_json` — record (de)serialization and the JSON payload type.
- `tdw-outbox` — source `OutboxRecord`s the stream is projected from.
- `tdw-event` *(dev only)* — sample events used in tests/examples.

## Quickstart

```rust
use tdw_cdc::CdcStream;
use tdw_event::sample_event;
use tdw_outbox::InMemoryOutbox;

let mut outbox = InMemoryOutbox::default();
outbox.append(sample_event("service"));
outbox.append(sample_event("worker"));

// Project the outbox into a CDC change stream.
let cdc = CdcStream::from_outbox(&outbox.pending_after(0));
assert_eq!(cdc.records.len(), 2);

// Tail everything after offset 1.
let tail = cdc.after(1);
assert_eq!(tail[0].offset, 2);
```

Run the worked example:

```text
cargo run -p tdw-cdc --example tdw-cdc-basic
```

## See also

- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — change-capture model and offset contract.
- `tdw-outbox` — the durable source of records.
