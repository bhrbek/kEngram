-- Fail-closed rollback for 0036 supersession transaction.
-- Roles are CLUSTER-GLOBAL in Postgres: never unconditional DROP ROLE after a
-- per-database object teardown when another database in the same cluster still
-- depends on the role (carl case_14 multi-DB root cause).

DO $$
DECLARE
  n bigint;
BEGIN
  SELECT count(*) INTO n FROM public.argus_source_event_supersession_receipts;
  IF n > 0 THEN
    RAISE EXCEPTION 'supersession_rollback_refused_receipts_present:%', n;
  END IF;
END$$;

DROP FUNCTION IF EXISTS public.supersede_argus_source_event(
  uuid, text, text, text, text, uuid, text, text, text, jsonb, text, text, text, text, text, text
);
DROP TRIGGER IF EXISTS argus_source_event_supersession_receipts_no_update
  ON public.argus_source_event_supersession_receipts;
DROP FUNCTION IF EXISTS public.argus_source_event_supersession_receipts_immutable();
DROP TABLE IF EXISTS public.argus_source_event_supersession_receipts;
-- B5: remove successor key-count helper if present (idempotent; residue-free rollback)
DROP FUNCTION IF EXISTS public.supersession_receipt_json_key_count(jsonb);
DELETE FROM public.corpus_hygiene_gate_settings
 WHERE principal_name = 'kengram_rt_supersession'
   AND producer_class = 'source_event_supersession';
DELETE FROM public.corpus_hygiene_producer_principals
 WHERE principal_name = 'kengram_rt_supersession';

-- Local privilege residue (this database only). Best-effort; missing objects OK.
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'kengram_rt_supersession') THEN
    BEGIN
      EXECUTE 'REVOKE ALL ON ALL TABLES IN SCHEMA public FROM kengram_rt_supersession';
    EXCEPTION WHEN undefined_object OR invalid_grant_operation THEN
      NULL;
    END;
    BEGIN
      EXECUTE 'REVOKE ALL ON ALL SEQUENCES IN SCHEMA public FROM kengram_rt_supersession';
    EXCEPTION WHEN undefined_object OR invalid_grant_operation THEN
      NULL;
    END;
    BEGIN
      EXECUTE 'REVOKE ALL ON ALL FUNCTIONS IN SCHEMA public FROM kengram_rt_supersession';
    EXCEPTION WHEN undefined_object OR invalid_grant_operation THEN
      NULL;
    END;
    BEGIN
      EXECUTE 'REVOKE USAGE ON SCHEMA public FROM kengram_rt_supersession';
    EXCEPTION WHEN undefined_object OR invalid_grant_operation THEN
      NULL;
    END;
  END IF;
END$$;

-- Drop the cluster-global role only when no remaining shared dependents exist
-- in ANY database of the cluster (pg_shdepend). Otherwise retain the role so a
-- sibling DB that still has 0036 applied keeps a working principal.
DO $$
DECLARE
  dep_count bigint;
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'kengram_rt_supersession') THEN
    RETURN;
  END IF;

  SELECT count(*)::bigint INTO dep_count
  FROM pg_catalog.pg_shdepend d
  JOIN pg_catalog.pg_authid a ON a.oid = d.refobjid
  WHERE a.rolname = 'kengram_rt_supersession';

  IF dep_count = 0 THEN
    DROP ROLE kengram_rt_supersession;
  ELSE
    RAISE NOTICE
      'kengram_rt_supersession retained after 0036 down: % cluster shared dependent(s) remain (multi-DB safe)',
      dep_count;
  END IF;
END$$;
