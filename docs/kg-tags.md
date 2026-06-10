# Knowledge Graph And Tags

G008 uses separate contracts for each responsibility:

- `tdw-kg`: entity catalog over the unified 50-kind taxonomy, relationships,
  neighbor queries, and real audited manual merges (alias union, edge rewiring,
  tombstones — knowledge-system A3). Merges remain explicitly approved; nothing
  auto-merges.
- `tdw-entity-resolver`: deterministic resolver candidates and explicit manual
  merge decisions.
- `tdw-tags`: tag definitions, parent DAG validation, assignments, TTL checks,
  provenance, and taxonomy stats.
- `tdw-tag-rules`: hot-reloadable rules with SQL injection guard and
  deterministic label/JSON/SQL-style predicates.
- `tdw-feature-store`: feature snapshots enriched with active tags.
- `tdw-service-api::kg_tag_sample`: KG query API, hybrid search tag filter,
  dbt model reference, agent tag interests, MCP tag tools, live tag bridge, and
  feature-store evidence.

The dbt model `meta_tag_assignments` provides tag lineage in the `system` schema.
The Postgres migration `20260521_0007_kg_tags_feature_store.sql` stores KG
entities, relationships, manual merge audit, tag definitions, assignments,
rules, and feature snapshots.
