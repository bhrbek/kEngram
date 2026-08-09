# 0037 — embedding_ann_projections schema-ledger true-up

**Lane:** kengram-prod-schema-ledger-drift-ann-projections  
**Owner:** diesel · **Reviewer:** neo · **Money path:** N  
**Date:** 2026-08-09

## Finding

| Fact | Evidence |
|------|----------|
| 0013 applied in prod, checksum matches source | Trinity GOLD INCOMPLETE-receipt 2026-08-09 |
| Relation absent from prod snapshot | Dual PG16.14 restores; zero `embedding_ann_projections` |
| No migration 0014..0036 drops it | `git grep` / migration tree on main |
| When/why | Out-of-band drop **2026-06-25** qwen ANN decommission; still present 2026-06-22 (BGE re-embed prep). Commit `467800c` documents posture `absent (decommissioned 2026-06-25)`. |

## Decision

**Recording migration 0037 (`DROP IF EXISTS`)** rather than a schema-baseline exception.

**Why:** exception leaves fresh `1..N` permanently creating a relation prod does not have; recording migration makes lineage close so catalog and ledger agree after 37 on both prod (no-op) and greenfield (create-then-drop). Does not recreate (out of scope).

## Verification

- `scripts/test-migration-0036-multi-db-down.sh` still **3/3** (migrate now through 37).
- No prod touch, no deploy.

## Local proofs (studio 2026-08-09)

### Fresh 1..N migrate (host PG17 + vector)

- `sqlx migrate run` applied **1..37** including `Applied 37/migrate drop embedding ann projections historical`.
- Post-migrate: `to_regclass(public.embedding_ann_projections)` = **false** (and coverage false).
- `migration_audit` has `0037_drop_embedding_ann_projections_historical`.
- Re-run migrate: no-op (ledger complete).

### Multi-db harness (official docker script)

- Script updated: all three migrate greps require `Applied 37/`.
- **Blocked on studio:** Docker Desktop hung (`docker info` does not return; same class as Trinity GOLD cleanup residue holding Virtualization handles). Harness not executed end-to-end here.

### Host multi-db equivalent (polluted shared cluster)

- Migrate two DBs through 37: OK; ANN absent both.
- Case1 watched-RED broken unconditional DROP ROLE: **PASS** (dependency class).
- Case2 fixed-down A retains role while B has objects: **PASS**.
- Case3 drop role after last DB: **cannot complete on this host** — pre-existing `pg_shdepend` on role from unrelated DB `kengram_narrow` (role correctly retained by multi-DB-safe down). Disposable docker cluster is required for clean case3.

Neo re-run of `./scripts/test-migration-0036-multi-db-down.sh` on a healthy docker host is the bar for official 3/3.
