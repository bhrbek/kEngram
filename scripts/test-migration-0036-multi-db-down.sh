#!/usr/bin/env bash
# Disposable multi-DB proof on an ephemeral local Postgres instance (random port).
# Requires the same Homebrew PostgreSQL major as `pg_config` plus the pgvector
# extension packaged for that major (CREATE EXTENSION vector must succeed).
# Never touches the shared host cluster or production.
#
# Proves:
#   1) unconditional DROP ROLE fails multi-DB (watched RED)
#   2) guarded down succeeds on DB-A while sibling DB-B still functions
#   3) guarded down on last DB removes the cluster role
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PG_BIN="${PG_BIN:-}"
if [[ -z "$PG_BIN" ]]; then
  if command -v pg_config >/dev/null 2>&1; then
    PG_BIN="$(dirname "$(pg_config --bindir 2>/dev/null || true)")"
  fi
fi
if [[ -z "$PG_BIN" || ! -x "${PG_BIN}/initdb" ]]; then
  if [[ -x /opt/homebrew/opt/postgresql@17/bin/initdb ]]; then
    PG_BIN=/opt/homebrew/opt/postgresql@17/bin
  else
    PG_BIN="$(dirname "$(command -v initdb)")"
  fi
fi
INITDB="$PG_BIN/initdb"
PG_CTL="$PG_BIN/pg_ctl"
PSQL="$PG_BIN/psql"
CREATEDB="$PG_BIN/createdb"
PG_CONFIG="$PG_BIN/pg_config"

for tool in "$INITDB" "$PG_CTL" "$PSQL" "$CREATEDB" sqlx python3; do
  if [[ "$tool" == /* ]]; then
    [[ -x "$tool" ]] || { echo "FAIL missing tool: $tool" >&2; exit 1; }
  else
    command -v "$tool" >/dev/null 2>&1 || { echo "FAIL missing tool: $tool" >&2; exit 1; }
  fi
done

DOWN_FIXED="$ROOT/migrations/rollback/0036_argus_source_event_supersession_transaction_down.sql"
DOWN_BROKEN_MUTANT="$ROOT/scripts/.tmp-0036-down-broken-drop-role.sql"
WORKDIR="${TMPDIR:-/tmp}/kengram-0036-multi-db-$$"
PORT="$(python3 - <<'PY'
import socket
s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()
PY
)"
export PATH="$PG_BIN:$PATH"

cleanup() {
  if [[ -n "${PGDATA:-}" && -d "${PGDATA:-}" ]]; then
    "$PG_CTL" -D "$PGDATA" -m fast stop >/dev/null 2>&1 || true
  fi
  rm -rf "$WORKDIR" "$DOWN_BROKEN_MUTANT" 2>/dev/null || true
}
trap cleanup EXIT

mkdir -p "$WORKDIR"
PGDATA="$WORKDIR/pgdata"
mkdir -p "$PGDATA"
"$INITDB" -D "$PGDATA" --auth-local=trust --auth-host=trust -U postgres >/tmp/kengram-0036-initdb.out 2>&1
"$PG_CTL" -D "$PGDATA" -o "-p $PORT -k $WORKDIR" -l "$WORKDIR/pg.log" start
for _ in $(seq 1 50); do
  if "$PSQL" -h 127.0.0.1 -p "$PORT" -U postgres -d postgres -c "SELECT 1" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done

# Preflight: pgvector must be loadable on this disposable instance.
if ! "$PSQL" -h 127.0.0.1 -p "$PORT" -U postgres -d postgres -v ON_ERROR_STOP=1 -c "CREATE EXTENSION vector;" >/tmp/kengram-0036-vector.out 2>&1; then
  echo "FAIL: disposable Postgres cannot CREATE EXTENSION vector." >&2
  echo "Need pgvector packaged for $($PG_CONFIG --version 2>/dev/null || echo unknown)." >&2
  echo "share=$($PG_CONFIG --sharedir 2>/dev/null) pkglib=$($PG_CONFIG --pkglibdir 2>/dev/null)" >&2
  cat /tmp/kengram-0036-vector.out >&2
  exit 1
fi

"$PSQL" -h 127.0.0.1 -p "$PORT" -U postgres -d postgres -v ON_ERROR_STOP=1 -c "CREATE USER kengram SUPERUSER LOGIN PASSWORD 'kengram';"
"$CREATEDB" -h 127.0.0.1 -p "$PORT" -U postgres -O kengram kengram_0036_multi_a
"$CREATEDB" -h 127.0.0.1 -p "$PORT" -U postgres -O kengram kengram_0036_multi_b

url_for() { echo "postgres://kengram:kengram@127.0.0.1:${PORT}/$1"; }
ADMIN_URL="postgres://kengram:kengram@127.0.0.1:${PORT}/postgres"
psql_admin() { "$PSQL" "$ADMIN_URL" -v ON_ERROR_STOP=1 "$@"; }
psql_db() {
  local db="$1"; shift
  "$PSQL" "$(url_for "$db")" -v ON_ERROR_STOP=1 "$@"
}

echo "== migrate both DBs to 0036 on ephemeral port $PORT =="
export SQLX_OFFLINE=true
( cd "$ROOT" && DATABASE_URL="$(url_for kengram_0036_multi_a)" sqlx migrate run --source migrations --no-dotenv ) >/tmp/kengram-0036-mig-a.out
( cd "$ROOT" && DATABASE_URL="$(url_for kengram_0036_multi_b)" sqlx migrate run --source migrations --no-dotenv ) >/tmp/kengram-0036-mig-b.out
grep -q "Applied 36/" /tmp/kengram-0036-mig-a.out || { echo "FAIL migrate A"; cat /tmp/kengram-0036-mig-a.out; exit 1; }
grep -q "Applied 36/" /tmp/kengram-0036-mig-b.out || { echo "FAIL migrate B"; cat /tmp/kengram-0036-mig-b.out; exit 1; }

role_exists() {
  psql_admin -tAc "SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='kengram_rt_supersession')" | tr -d '[:space:]'
}
fn_exists() {
  local db="$1"
  psql_db "$db" -tAc "SELECT to_regprocedure('public.supersede_argus_source_event(uuid,text,text,text,text,uuid,text,text,text,jsonb,text,text,text,text,text,text)') IS NOT NULL" | tr -d '[:space:]'
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
print("wrote mutant", sys.argv[2])
PY

set +e
psql_db kengram_0036_multi_a -f "$DOWN_BROKEN_MUTANT" >/tmp/kengram-0036-broken-down.out 2>&1
broken_rc=$?
set -e
if [[ "$broken_rc" -eq 0 ]]; then
  echo "FAIL: broken down unexpectedly succeeded on multi-DB cluster" >&2
  cat /tmp/kengram-0036-broken-down.out >&2
  exit 1
fi
if ! grep -Eiq 'depend|cannot be dropped|being used by' /tmp/kengram-0036-broken-down.out; then
  echo "FAIL: broken down did not fail with dependency class" >&2
  cat /tmp/kengram-0036-broken-down.out >&2
  exit 1
fi
echo "PASS watched-RED broken-down multi-DB dependency (rc=$broken_rc)"

# Rebuild A cleanly
psql_admin -c "DROP DATABASE IF EXISTS kengram_0036_multi_a WITH (FORCE);"
psql_admin -c "CREATE DATABASE kengram_0036_multi_a OWNER kengram;"
( cd "$ROOT" && DATABASE_URL="$(url_for kengram_0036_multi_a)" sqlx migrate run --source migrations --no-dotenv ) >/tmp/kengram-0036-mig-a2.out
grep -q "Applied 36/" /tmp/kengram-0036-mig-a2.out

echo "== fixed down on A while B still has 0036 =="
psql_db kengram_0036_multi_a -f "$DOWN_FIXED" >/tmp/kengram-0036-fixed-down-a.out 2>&1
test "$(fn_exists kengram_0036_multi_a)" = "f"
test "$(fn_exists kengram_0036_multi_b)" = "t"
test "$(role_exists)" = "t"
echo "PASS fixed-down A: A clean, B function+role retained"

echo "== fixed down on B (last remaining DB) =="
psql_db kengram_0036_multi_b -f "$DOWN_FIXED" >/tmp/kengram-0036-fixed-down-b.out 2>&1
test "$(fn_exists kengram_0036_multi_b)" = "f"
test "$(role_exists)" = "f"
echo "PASS fixed-down B: cluster role removed when last dependent gone"

echo "PASS kengram-0036-multi-db-down selected=3 executed=3 failed=0 skipped=0"
