-- Migration 0037: record historical out-of-band decommission of qwen3 ANN projections
--
-- FINDING (Trinity GOLD 2026-08-09, Knox board kengram-prod-schema-ledger-drift-ann-projections):
--   * Migration 0013 (qwen3_ann_projection) is recorded applied in prod with a checksum that
--     matches the exact source file (creates embedding_ann_projections + coverage + HNSW).
--   * The relation is ABSENT from prod (immutable snapshot dual restores).
--   * No later migration (0014..0036) DROP-s the relation.
--   * Therefore the drop was out-of-band manual DDL, not ledgered.
--
-- WHEN / WHY (repo archive — pg logs unavailable):
--   * 2026-06-22: knox bge-m3-reembed prep still saw embedding_ann_projections live in
--     kengram_prod as the qwen3 halfvec-3072 latency path (agents/knox import of
--     kengram-yeti-bge-m3-reembed-2026-06-22.md).
--   * 2026-06-25: decommission date recorded in tree at 467800c
--     ("backup: observe ANN projection coverage posture instead of requiring it"):
--     "prod dropped the qwen3 projection tables out-of-chain in the 2026-06-25
--     decommission". corpus_stats now records ann_projection_posture
--     present | absent (decommissioned 2026-06-25).
--   * Context: qwen3-4096 + halfvec ANN path retired as BGE-M3 sidecars (0016+) became
--     the serving path; drop was operator DDL during that transition, never a migration.
--
-- CHOICE: recording migration (DROP IF EXISTS), not a schema-baseline exception.
--   Why: fresh 1..N must match prod reality (relation absent). Leaving 0013 as the last
--   word on the relation forces every greenfield migrate to create a table prod no longer
--   has. An exception only documents the drift; a recording migration closes the lineage
--   so ledger + catalog agree after 0037 on both prod (no-op drop) and fresh installs
--   (0013 create then 0037 drop). Does NOT recreate the relation (out of scope).
--
-- Idempotent: safe to apply on prod where tables are already gone.

SET lock_timeout = '5s';
SET statement_timeout = '5min';

DROP INDEX IF EXISTS embedding_ann_projection_qwen3_embedding_halfvec_3072_hnsw;
DROP INDEX IF EXISTS embedding_ann_projection_qwen3_3072_hnsw;
DROP INDEX IF EXISTS embedding_ann_projections_model_target_idx;
DROP TABLE IF EXISTS embedding_ann_projection_coverage;
DROP TABLE IF EXISTS embedding_ann_projections;

INSERT INTO migration_audit (migration, rows_touched, notes)
VALUES (
    '0037_drop_embedding_ann_projections_historical',
    0,
    'Record 2026-06-25 out-of-band decommission of qwen3 ANN projection sidecars (embedding_ann_projections + coverage). Created by 0013; dropped outside the ledger during qwen→BGE transition. DROP IF EXISTS aligns fresh 1..N with prod. Does not recreate.'
);
