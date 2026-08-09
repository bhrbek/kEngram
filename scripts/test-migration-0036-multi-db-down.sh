#!/usr/bin/env bash
# Disposable multi-DB proof for 0036 down: cluster-global role safety.
# Uses the same pgvector/pgvector:pg16 disposable-container pattern as
# scripts/test-migration-0035-reconciliation.sh — does NOT depend on a host
# Homebrew pgvector install.
#
# Never targets production. Never uses the shared host Postgres cluster.
set -euo pipefail
export LC_ALL=C

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="pgvector/pgvector:pg16"
SELECTED=3
EXECUTED=0
CONTAINER="kengram-0036-multi-$$-${RANDOM}"
CONTAINER_ID=""
DOWN_FIXED="$ROOT/migrations/rollback/0036_argus_source_event_supersession_transaction_down.sql"
DB_A="kengram_0036_multi_a"
DB_B="kengram_0036_multi_b"
PGUSER="kengram"  # superuser for disposable cluster; migrations OWNER TO kengram (neo F2a)
PGPASS="acceptance-only"

fail() {
  printf 'FAIL kengram-0036-multi-db-down: %s\n' "$*" >&2
  exit 1
}

pass_case() {
  EXECUTED=$((EXECUTED + 1))
  printf 'PASS kengram-0036-multi-db-down case=%s selected=%s executed=%s\n' "$1" "$SELECTED" "$EXECUTED"
}

for tool in docker sqlx python3 awk grep sed shasum; do
  command -v "$tool" >/dev/null 2>&1 || fail "missing required tool: $tool"
done
test -f "$DOWN_FIXED" && test ! -L "$DOWN_FIXED" || fail "missing fixed down file: $DOWN_FIXED"

WORK="$(mktemp -d /tmp/kengram-0036-multi.XXXXXX)"
case "$WORK" in
  /tmp/kengram-0036-multi.*) ;;
  *) fail "unexpected temporary directory: $WORK" ;;
esac
test -d "$WORK" && test ! -L "$WORK" || fail "temporary directory is not a real directory"

DOWN_BROKEN_MUTANT="$WORK/broken-unconditional-drop-role.sql"

cleanup() {
  prior_rc=$?
  cleanup_rc=0
  trap - EXIT INT TERM

  if test -n "$CONTAINER_ID" && docker inspect "$CONTAINER" >/dev/null 2>&1; then
    actual_id="$(docker inspect --format '{{.Id}}' "$CONTAINER" 2>/dev/null || true)"
    actual_label="$(docker inspect --format '{{index .Config.Labels "io.yetiwerks.kengram-0036-multi"}}' "$CONTAINER" 2>/dev/null || true)"
    if test "$actual_id" = "$CONTAINER_ID" && test "$actual_label" = "$CONTAINER"; then
      docker stop -t 5 "$CONTAINER" >/dev/null 2>&1 || cleanup_rc=1
    else
      printf 'FAIL cleanup refused unexpected container identity name=%s\n' "$CONTAINER" >&2
      cleanup_rc=1
    fi
  fi

  if test -d "$WORK" && test ! -L "$WORK"; then
    if test -n "$(find "$WORK" -type l -print -quit 2>/dev/null)"; then
      printf 'FAIL cleanup refused temporary tree containing a symlink: %s\n' "$WORK" >&2
      cleanup_rc=1
    else
      /bin/rm -rf -- "$WORK" || cleanup_rc=1
    fi
  fi

  if test "$prior_rc" -eq 0 && test "$cleanup_rc" -ne 0; then
    exit "$cleanup_rc"
  fi
  exit "$prior_rc"
}
trap cleanup EXIT INT TERM

case "$CONTAINER" in
  kengram-0036-multi-[0-9]*-[0-9]*) ;;
  *) fail "unexpected container name: $CONTAINER" ;;
esac

CONTAINER_ID="$(docker run --rm -d \
  --name "$CONTAINER" \
  --label "io.yetiwerks.kengram-0036-multi=$CONTAINER" \
  -e POSTGRES_USER="$PGUSER" \
  -e POSTGRES_PASSWORD="$PGPASS" \
  -e POSTGRES_DB="$DB_A" \
  -p 127.0.0.1::5432 \
  "$IMAGE")" || fail "disposable PostgreSQL container failed to start"
case "$CONTAINER_ID" in
  [0-9a-f][0-9a-f]*) ;;
  *) fail "invalid disposable container id" ;;
esac

ready=0
attempt=0
while test "$attempt" -lt 40; do
  if docker exec "$CONTAINER" pg_isready -U "$PGUSER" -d "$DB_A" >/dev/null 2>&1; then
    ready=1
    break
  fi
  attempt=$((attempt + 1))
  sleep 1
done
test "$ready" -eq 1 || fail "disposable PostgreSQL did not become ready"

# Prove pgvector is provisioned inside the container (not host Homebrew).
docker exec "$CONTAINER" psql -X -v ON_ERROR_STOP=1 -U "$PGUSER" -d "$DB_A" \
  -c "CREATE EXTENSION IF NOT EXISTS vector;" >/dev/null \
  || fail "pgvector/pgvector image cannot CREATE EXTENSION vector"

PORT_LINE="$(docker port "$CONTAINER" 5432/tcp)" || fail "disposable port lookup failed"
PORT="${PORT_LINE##*:}"
case "$PORT" in
  ''|*[!0-9]*) fail "invalid disposable PostgreSQL port: $PORT" ;;
esac

url_for() {
  printf 'postgres://%s:%s@127.0.0.1:%s/%s\n' "$PGUSER" "$PGPASS" "$PORT" "$1"
}

# Second database on the same cluster
docker exec "$CONTAINER" psql -X -v ON_ERROR_STOP=1 -U "$PGUSER" -d "$DB_A" \
  -c "CREATE DATABASE ${DB_B} OWNER ${PGUSER};" >/dev/null \
  || fail "CREATE DATABASE B failed"

export SQLX_OFFLINE=true

echo "== migrate both DBs through 0037 (includes historical ANN drop) =="
( cd "$ROOT" && DATABASE_URL="$(url_for "$DB_A")" sqlx migrate run --source migrations --no-dotenv ) >"$WORK/mig-a.out" 2>&1 \
  || { cat "$WORK/mig-a.out" >&2; fail "migrate A failed"; }
( cd "$ROOT" && DATABASE_URL="$(url_for "$DB_B")" sqlx migrate run --source migrations --no-dotenv ) >"$WORK/mig-b.out" 2>&1 \
  || { cat "$WORK/mig-b.out" >&2; fail "migrate B failed"; }
grep -q "Applied 37/" "$WORK/mig-a.out" || fail "migrate A missing Applied 37"
grep -q "Applied 37/" "$WORK/mig-b.out" || fail "migrate B missing Applied 37"

role_exists() {
  docker exec "$CONTAINER" psql -X -U "$PGUSER" -d "$DB_A" -At \
    -c "SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='kengram_rt_supersession')"
}

fn_exists() {
  local db="$1"
  docker exec "$CONTAINER" psql -X -U "$PGUSER" -d "$db" -At \
    -c "SELECT to_regprocedure('public.supersede_argus_source_event(uuid,text,text,text,text,uuid,text,text,text,jsonb,text,text,text,text,text,text)') IS NOT NULL"
}

psql_db_file() {
  local db="$1"
  local file="$2"
  local out="$3"
  docker exec -i "$CONTAINER" psql -X -v ON_ERROR_STOP=1 -U "$PGUSER" -d "$db" <"$file" >"$out" 2>&1
}

echo "== watched RED: legacy unconditional DROP ROLE after multi-DB apply =="
python3 - "$DOWN_FIXED" "$DOWN_BROKEN_MUTANT" <<'PY'
from pathlib import Path
import sys
fixed = Path(sys.argv[1]).read_text()
marker = "-- Drop the cluster-global role only when"
if marker not in fixed:
    raise SystemExit("marker missing in fixed down")
head = fixed.split(marker)[0]
Path(sys.argv[2]).write_text(head + "DROP ROLE IF EXISTS kengram_rt_supersession;\n")
PY

set +e
psql_db_file "$DB_A" "$DOWN_BROKEN_MUTANT" "$WORK/broken-down.out"
broken_rc=$?
set -e
if test "$broken_rc" -eq 0; then
  cat "$WORK/broken-down.out" >&2
  fail "broken down unexpectedly succeeded on multi-DB cluster"
fi
if ! grep -Eiq 'depend|cannot be dropped|being used by' "$WORK/broken-down.out"; then
  cat "$WORK/broken-down.out" >&2
  fail "broken down did not fail with dependency class"
fi
pass_case watched-RED-broken-unconditional-drop

# Rebuild A cleanly (broken down is multi-statement; may partially apply).
# DROP DATABASE cannot run inside a multi-statement implicit transaction (neo F2b) —
# issue DROP and CREATE as separate psql requests.
docker exec "$CONTAINER" psql -X -v ON_ERROR_STOP=1 -U "$PGUSER" -d "$DB_B" \
  -c "DROP DATABASE IF EXISTS ${DB_A} WITH (FORCE);" >/dev/null \
  || fail "rebuild A DROP failed"
docker exec "$CONTAINER" psql -X -v ON_ERROR_STOP=1 -U "$PGUSER" -d "$DB_B" \
  -c "CREATE DATABASE ${DB_A} OWNER ${PGUSER};" >/dev/null \
  || fail "rebuild A CREATE failed"
( cd "$ROOT" && DATABASE_URL="$(url_for "$DB_A")" sqlx migrate run --source migrations --no-dotenv ) >"$WORK/mig-a2.out" 2>&1 \
  || { cat "$WORK/mig-a2.out" >&2; fail "re-migrate A failed"; }
grep -q "Applied 37/" "$WORK/mig-a2.out" || fail "re-migrate A missing Applied 37"

echo "== fixed down on A while B still has supersession 0036 objects =="
psql_db_file "$DB_A" "$DOWN_FIXED" "$WORK/fixed-down-a.out" || {
  cat "$WORK/fixed-down-a.out" >&2
  fail "fixed down A failed"
}
test "$(fn_exists "$DB_A")" = "f" || fail "A function still present after fixed down"
test "$(fn_exists "$DB_B")" = "t" || fail "B function missing after A down"
test "$(role_exists)" = "t" || fail "role dropped while B still depends"
pass_case fixed-down-A-retains-role-for-B

echo "== fixed down on B (last remaining DB) =="
psql_db_file "$DB_B" "$DOWN_FIXED" "$WORK/fixed-down-b.out" || {
  cat "$WORK/fixed-down-b.out" >&2
  fail "fixed down B failed"
}
test "$(fn_exists "$DB_B")" = "f" || fail "B function still present after fixed down"
test "$(role_exists)" = "f" || fail "role still present after last dependent down"
pass_case fixed-down-B-removes-role

test "$EXECUTED" -eq "$SELECTED" || fail "executed=$EXECUTED selected=$SELECTED mismatch"
printf 'PASS kengram-0036-multi-db-down selected=%s executed=%s failed=0 skipped=0\n' "$SELECTED" "$EXECUTED"
