# tdw-cdc — Architecture

## Module map

Single `lib.rs`:

| Item | Role |
| --- | --- |
| `CdcRecord` | One projected change: `offset`, `event_id`, `event_type`, `payload`. |
| `CdcStream` | Ordered collection of `CdcRecord`s. |
| `CdcStream::from_outbox` | Project `&[OutboxRecord]` into a stream. |
| `CdcStream::after` | Return records with `offset` strictly greater than a cursor. |

## Key types and traits

- Both `CdcRecord` and `CdcStream` derive `Clone, Debug, PartialEq, Eq, Serialize,
  Deserialize`, so a stream can itself be serialized for transport or snapshotting.
- `payload` is a `serde_json::Value` — CDC is payload-agnostic; it carries the
  envelope body verbatim without re-typing it.

## Change-capture model

The platform uses the **transactional outbox** pattern:

```
domain write + outbox.append(envelope)   (same transaction)
                     │
                     ▼
        tdw-outbox: OutboxRecord { sequence, envelope, status }
                     │  pending_after(cursor)
                     ▼
   tdw-cdc: CdcStream::from_outbox(&records)
                     │   maps sequence ▶ offset, lifts event_id/event_type/payload
                     ▼
        CdcStream { records: [CdcRecord, ...] }
                     │   after(offset)
                     ▼
            consumer tail (resumable from last offset)
```

`from_outbox` is a 1:1 field projection — it does not filter or reorder; ordering
and pending/dispatched filtering are the outbox's responsibility
(`pending_after`). CDC only re-shapes records into a consumer-facing view and
provides the offset-cursor read (`after`).

## Invariants

- **Offset = outbox sequence.** Offsets are monotonic and stable; a consumer that
  remembers its last offset can always resume with `after(last_offset)`.
- `after(offset)` is **exclusive** of `offset` (strictly `>`), so re-tailing with
  the last processed offset never re-delivers the same record.
- The projection preserves input order; `records[i].offset` mirrors the source
  outbox ordering.
- Pure and deterministic: no I/O, no clock, no global state. Durability and
  transactional guarantees live in `tdw-outbox`, not here.
