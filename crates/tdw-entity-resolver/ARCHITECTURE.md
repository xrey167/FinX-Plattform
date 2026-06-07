# tdw-entity-resolver — Architecture

## Module map

| File | Contents |
| --- | --- |
| `lib.rs` | Symbol/identifier resolution, manual-merge decisions, grammar checks. |
| `openfigi.rs` | Pure parser for `OpenFIGI` `/v3/mapping` response bodies. |

### `lib.rs` items

| Item | Role |
| --- | --- |
| `ResolveCandidate` | A match: `entity_id`, `score: u8`, `reason`. |
| `MergeDecision` | `source`, `target`, `approved`, `audited`. |
| `IdentifierRecord` | Crosswalk row: `scheme`, `value`, `instrument_id`. |
| `ResolveError` | `InvalidSymbol`, `InvalidMergeEndpoint`, `InvalidIdentifier`. |
| `resolve_symbol` / `try_resolve_symbol` | Ticker/alias resolution. |
| `resolve_by_identifier` / `try_resolve_by_identifier` | Crosswalk resolution. |
| `manual_merge_decision` / `try_manual_merge_decision` | Audited merge record. |

### `openfigi.rs` items

`OpenFigiMapping` (figi/name/ticker/exch_code), `OpenFigiParseError`
(`InvalidJson`, `UnexpectedShape`), `parse_openfigi_mapping`.

## Key types and traits

- `ResolveCandidate`, `MergeDecision`, `IdentifierRecord`, `OpenFigiMapping` all
  derive `Clone, Debug, PartialEq, Eq, Serialize, Deserialize`.
- `ResolveError` is a `Copy`, `serde`-serializable enum.
- The `try_*` functions are the validating entry points; the bare functions are
  thin wrappers returning `unwrap_or_default()` for ergonomic best-effort use.

## Resolution / data flow

```
resolve_symbol(symbol, entities):
    is_symbol(symbol)?                       // grammar
    normalized = symbol.to_ascii_uppercase()
    keep entities where kind == Instrument
        AND (label ~= normalized OR any alias ~= normalized)   // case-insensitive
    ▶ [ResolveCandidate { score: 100, reason: "exact symbol or alias match" }]

resolve_by_identifier(scheme, value, records):
    is_identifier_scheme(scheme)? && is_identifier_value(value)?
    keep records where scheme ~= scheme AND value ~= trim(value)  // case-insensitive
    ▶ [ResolveCandidate { score: 100, reason: "exact <SCHEME> identifier match" }]

manual_merge_decision(source, target, approved):
    ▶ MergeDecision { audited: true, .. }     // try_ variant rejects self-merge
```

`openfigi::parse_openfigi_mapping` is independent of resolution: it walks the
top-level job array, skips jobs that carry a `warning`/`error` (no `data`), and
emits one `OpenFigiMapping` per `data` entry that has a `figi` (other fields
optional).

## Invariants

- **Symbol grammar**: non-empty, no surrounding whitespace, ASCII alphanumeric
  plus `.`, `-`, `_`. Matching is case-insensitive (input upper-cased).
- **Identifier scheme/value grammar**: non-empty after trim; scheme allows `_`/`-`,
  value allows `.`/`-`. Both matched case-insensitively; path-like inputs
  (`../secret`) are rejected as `InvalidIdentifier`.
- **Merge endpoints**: both must be valid entity ids and must differ; a self-merge
  is `InvalidMergeEndpoint`.
- Every produced candidate is an **exact** match with `score = 100`; this resolver
  models exact resolution, not fuzzy ranking.
- `MergeDecision.audited` is always `true` — manual merges are always recorded.
- `parse_openfigi_mapping` does **no** network I/O; it only parses a supplied body
  and never fails on missing optional fields (only on invalid JSON / non-array top
  level).
