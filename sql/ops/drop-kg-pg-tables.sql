-- Operator script: drop legacy Postgres KG/tag tables (knowledge-system F1)
--
-- WHEN TO RUN
-- -----------
-- Run this ONCE, manually, after deploying knowledge-system F1 (the graph
-- cutover). These tables were created by migration
-- 20260521_0007_kg_tags_feature_store.sql and are no longer used by any
-- runtime path: the KG, tags, and inference surfaces now run exclusively on
-- the graph backend (Bolt/Memgraph in production, InMemoryGraphEngine in
-- dev/test). The `feature_snapshot` table is kept — it backs the feature-store
-- evidence path and is still written to by tdw-feature-store.
--
-- WHAT IS DROPPED
-- ---------------
--   kg_entity           — canonical entity rows (replaced by graph nodes)
--   kg_relationship     — entity edges (replaced by graph edges)
--   kg_merge_audit      — manual merge audit log (no replacement; merges are
--                         now graph-side operations)
--   tag_definition      — tag type registry (replaced by graph tag-node layer)
--   tag_assignment      — entity→tag links (replaced by graph tag edges)
--   tag_rule            — hot-reloadable auto-tagging rules (in-memory only,
--                         loaded from the file system at startup)
--
-- WHAT IS KEPT
-- ------------
--   feature_snapshot    — feature-store evidence (still active)
--
-- SAFETY CHECKLIST
-- ----------------
-- 1. Verify no application writes to these tables: search for INSERT/UPDATE
--    statements against the table names across all deployed service versions.
-- 2. Take a Postgres dump (pg_dump --schema-only) before running so you have
--    the DDL on record.
-- 3. If you need the data for audit purposes, export it first:
--      COPY kg_entity TO '/tmp/kg_entity_export.csv' CSV HEADER;
--      (repeat for each table)
-- 4. Run in a transaction and ROLLBACK if anything looks wrong before COMMIT.
--
-- HOW TO RUN
-- ----------
--   psql $TDW_DATABASE_URL -f sql/ops/drop-kg-pg-tables.sql
--
-- Or interactively:
--   BEGIN;
--   \i sql/ops/drop-kg-pg-tables.sql
--   -- verify, then:
--   COMMIT;

BEGIN;

DROP TABLE IF EXISTS tag_rule CASCADE;
DROP TABLE IF EXISTS tag_assignment CASCADE;
DROP TABLE IF EXISTS tag_definition CASCADE;
DROP TABLE IF EXISTS kg_merge_audit CASCADE;
DROP TABLE IF EXISTS kg_relationship CASCADE;
DROP TABLE IF EXISTS kg_entity CASCADE;

COMMIT;
