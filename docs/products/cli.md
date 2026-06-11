# CLI (catalog-derived) — quickstart

`tdw-cli` exposes a **catalog-derived** command tree: every route in the
endpoint catalog (`tdw-endpoint-catalog`) — both `Fetch` and `Compute` (e.g.
`technical/*`) routes — becomes a nested subcommand path, mirroring the OpenBB
CLI's `equity price historical` shape. The tree is built **at runtime from the
catalog**, so a new catalog route is callable from the CLI with no CLI code
change. There is no per-route handler: a leaf command submits `Op::FetchData`
over the daemon's existing length-delimited TCP `OpEnvelope` path (the same
transport `run-query` uses) and renders the terminal `EventMsg`.

```text
tdw equity price historical --symbol AAPL --provider yahoo
tdw technical sma --symbol AAPL --length 20
tdw routes                 # list every catalog route with kind + providers
tdw routine list           # list recorded routines
```

> The daemon must be listening (default `127.0.0.1:7878`; override per command
> with `--addr`). A live end-to-end run needs a running daemon; the catalog
> command-tree, argument mapping, table/CSV rendering, and routine record/run
> layers are covered by unit tests, and the offline `--smoke` path exercises the
> full provider→storage→read round trip without a daemon.

## Command surface

| Command | Description |
|---------|-------------|
| `tdw <route...> [flags]` | Resolve a catalog route, submit `Op::FetchData`, render rows |
| `tdw routes` | List every catalog route with its kind (`fetch`/`compute`) and providers |
| `tdw routine record <name> -- <route...>` | Run a command and append its `OpEnvelope` to `.tdw/routines/<name>.jsonl` |
| `tdw routine run <name> [--var k=v]` | Re-submit each recorded envelope (fresh op ids; `${k}` substitution) |
| `tdw routine list` | List recorded routine names |
| `tdw run-query "<sql>"` | (Legacy, unchanged) submit `Op::RunQuery` |
| `tdw --smoke [SYMBOL]` | (Legacy, unchanged) offline end-to-end smoke |

`<route...>` is a catalog route written as space-separated segments, e.g.
`equity price historical`, `crypto price historical`, `economy cpi`,
`fixedincome government treasury_auctions`. Run `tdw routes` to enumerate them.

## How schema maps to arguments

Each leaf's value flags are generated from the route's `params_schema`
properties:

- every property becomes a `--<name>` flag, typed by its JSON-schema `type`
  (`integer`/`number` → numeric, `boolean` → presence flag, everything else —
  including enum / `$ref` shapes such as `interval` and `period` — → string token
  the daemon parses);
- `--symbol` is always available (the universal instrument key the provider
  fetchers read out of the params object; it is not part of the shared
  `StandardParams` schema, so it is added unconditionally);
- `--provider` is available on **Fetch** routes only, to pin one candidate
  (default: the catalog's declaration-order fallback, offline/keyless first);
- `--chart` is forwarded as `"chart": true` into the params object. The chart
  envelope slot is **G014** (in-flight); a dispatcher that does not yet read the
  key simply ignores it, so this is forward-compatible.

Only the flags you actually pass are inserted into the params object — the daemon
fills defaults. Inspect any leaf with `--help`:

```text
$ tdw equity price historical --help
      --symbol <symbol>          Instrument symbol / identifier passed in params
      --provider <provider>      Pin a specific provider candidate
      --start_date <start_date>
      --end_date <end_date>
      --interval <interval>
      --period <period>
      --limit <limit>
      --chart
      --json
      --export <export>          [possible values: csv, json]
      --out <out>
      --addr <addr>              [default: 127.0.0.1:7878]
```

## Output and export

- **Default**: an aligned plain-text table of the result rows (column widths are
  derived from the row keys; rendering is hand-rolled, no table dependency is
  pulled into the CLI).
- `--json`: prints the raw terminal-event `result` (the `{ evidence, result:
  <ResultEnvelope> }` body) as pretty JSON.
- `--export csv|json [--out PATH]`: writes the rows to a file (default
  `<route_with_underscores>.<ext>`). CSV uses hand-rolled RFC-4180 escaping.

**Export scope**: CSV and JSON only. Parquet and XLSX export (and the
`export_directory`-style config) are owned by gap-matrix item **L5.4**
(`tdw-table-format` / `tdw-storage-parquet`) and are intentionally **out of
scope** for the CLI transport layer.

## Routines (event-spine replay)

A routine is a replayable script of `OpEnvelope`s stored as JSONL under
`.tdw/routines/<name>.jsonl` in the working directory. This is **local-file state
only** — there is no daemon-side routine registry — so a routine file is portable
and inspectable (plain JSON envelopes you can read, diff, and hand-edit). This is
a stricter, more faithful replay surface than OpenBB's free-text routine scripts:
each step is the exact wire envelope that was submitted.

```text
# Record: runs the command now AND appends its envelope to the routine file.
tdw routine record daily -- equity price historical --symbol AAPL --provider yahoo

# Replay: re-submits each stored envelope with a fresh op/session id.
tdw routine run daily

# Replay with substitution: every ${sym} in the stored params becomes MSFT.
tdw routine record byvar -- equity price quote --symbol '${sym}'
tdw routine run byvar --var sym=MSFT
```

Substitution is a literal `${key}` → `value` replace over each step's serialized
params object, so it applies uniformly to string, number, and nested values
without a templating dependency.

## Relationship to the other surfaces

The CLI, the [REST surface](./rest-api.md), the OpenAPI document, and the
Workspace bridges all derive from the **same** `tdw-endpoint-catalog` table and
dispatch through the **same** policy-guarded `Op::FetchData` path. The CLI is a
thin transport + rendering layer over that shared spine; it adds no business
logic of its own.
