# tdw-taxonomy

The unified entity taxonomy shared by the agent plane and the warehouse plane — the
single source of truth for entity classification across the platform.

## What lives here

- **`kind`** — the closed registry of 50 classified [`EntityKind`]s and their manifest
  [`Group`]s (core, tools, orchestration, knowledge, governance, infra, meta, domain).
  The `domain` group holds the warehouse entities (instrument, account, strategy,
  dataset, provider, symbol, venue).
- **`facets`** — cross-cutting facets attached to specific kinds: [`DataFacets`]
  (plane / materialization / `as_of` / validation gate) and [`EvalFacets`] (ML rigor for
  evaluating agent skills).
- **`origin`** — the orthogonal [`Origin`] (tier × source) classification of a kind
  itself (not data lineage; lineage is a `provenance` field elsewhere).

## Design constraints

- **Leaf crate**: pure data types, no I/O, no platform dependencies — every layer
  (knowledge graph, tags, retrieval, agents, MCP) can depend on it without cycles.
- **Declaration order is load-bearing**: `EntityKind`'s derived `Ord` follows variant
  declaration order, which drives `EntityKind::ALL` and deterministic registry
  iteration. New groups are appended, never sorted into place.
- **Serde compatibility**: kinds serialize to the registry's lowercase token convention
  (`agentrouter`, `knowledgegraph`, `resourcedefinition`, `instrument`, …).

`tdw-agent` re-exports these types from their original module paths
(`tdw_agent::{kind, facets, base}`), so pre-A1 consumers compile unchanged.

## Example

```text
EntityKind::Instrument.group()        // Group::Domain
EntityKind::Memory.is_data_kind()     // true (knowledge group carries DataFacets)
EntityKind::Agent.is_autonomy_capable() // true — and only Agent
```
