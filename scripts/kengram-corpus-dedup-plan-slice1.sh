#!/usr/bin/env bash
# kengram corpus-dedup plan — D3 slice-1 exact-duplicate allowlist (plan-only)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec python3 "$ROOT/scripts/corpus_dedup/plan_exact_dup_slice1.py" "$@"
