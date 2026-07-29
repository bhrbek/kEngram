#!/usr/bin/env python3
"""
kengram corpus-dedup plan — D3 slice-1 EXACT-DUPLICATE only (plan-only).

Frozen allowlist (Knox 2026-07-29):
  - gate-ledger exact-duplicate fingerprints (event_count >= 2)
  - gold-100 passage exact-dup clusters (29 clusters; items protected)

No --apply. Gold thought_ids and gold corpus items are untouchable.
pass@10: records baseline protocol; does not mutate retrieval index.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load_jsonl(path: Path) -> list[dict[str, Any]]:
    rows = []
    with path.open() as f:
        for line in f:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    return rows


def load_gold_protection(gold_path: Path | None) -> dict[str, Any]:
    """252 gold items: never keeper-retract; never appear as retract_duplicate."""
    protected_thought_ids: set[str] = set()
    by_passage: dict[str, list[str]] = defaultdict(list)
    n_items = 0
    if not gold_path or not gold_path.exists():
        return {
            "gold_path": str(gold_path) if gold_path else None,
            "gold_items_total": 0,
            "protected_thought_ids": [],
            "passage_exact_dup_clusters": 0,
            "warning": "gold corpus missing — protection set empty",
        }
    with gold_path.open() as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            qid = str(row.get("id") or row.get("query_id") or "")
            gold_list = row.get("gold")
            if not isinstance(gold_list, list):
                gold_list = [row]
            for item in gold_list:
                if not isinstance(item, dict):
                    continue
                n_items += 1
                tid = item.get("thought_id")
                if tid:
                    protected_thought_ids.add(str(tid))
                text = item.get("passage") or item.get("text") or ""
                if text:
                    ph = sha256_bytes(text.encode("utf-8"))
                    by_passage[ph].append(qid or f"item-{n_items}")
    passage_clusters = []
    for ph, qids in by_passage.items():
        uniq = sorted(set(qids))
        if len(qids) >= 2:
            passage_clusters.append(
                {"passage_sha256": ph, "n": len(qids), "query_ids": uniq}
            )
    passage_clusters.sort(key=lambda c: -c["n"])
    return {
        "gold_path": str(gold_path),
        "gold_sha256": sha256_file(gold_path),
        "gold_items_total": n_items,
        "protected_thought_ids": sorted(protected_thought_ids),
        "protected_thought_id_count": len(protected_thought_ids),
        "passage_exact_dup_clusters": len(passage_clusters),
        "passage_clusters": passage_clusters[:29],
    }



def plan_from_gate_ledger(
    clusters: list[dict[str, Any]],
    protected_thought_ids: set[str],
    min_events: int,
) -> list[dict[str, Any]]:
    """
    Each exact-dup fingerprint maps to one stored thought content; duplicates are
    multi-event ingest of the same fingerprint. Slice-1 plan records cluster for
    review; apply_eligible=false for any gold-protected thought; no retract without
    multi-thought cluster (true multi-id exact content).
    """
    actions: list[dict[str, Any]] = []
    for c in clusters:
        n = int(c.get("event_count") or c.get("n") or 0)
        if n < min_events:
            continue
        fp = (
            c.get("content_fingerprint")
            or c.get("fingerprint_hex")
            or c.get("fingerprint")
            or ""
        )
        mids = c.get("matched_thought_ids") or []
        thought_id = (
            c.get("thought_id")
            or c.get("keeper_thought_id")
            or (mids[0] if mids else None)
        )
        scopes = c.get("scopes") or []
        scope = c.get("scope") or (scopes[0] if scopes else None)
        sources = c.get("sources") or []
        tid_set = set(str(x) for x in (mids or ([] if not thought_id else [thought_id])))
        gold_hit = bool(tid_set & protected_thought_ids)
        decision = "review_only_gold_protected" if gold_hit else "exact_duplicate_cluster_report"
        # Slice-1: plan-only inventory; multi-event same fingerprint is not multi-thought
        # retract until thought-id multiplicity proven. apply_eligible stays false.
        actions.append(
            {
                "kind": "exact_duplicate_cluster",
                "content_fingerprint": fp,
                "event_count": n,
                "thought_id": thought_id,
                "scope": scope,
                "sources": sources[:20],
                "source_count": len(sources) if isinstance(sources, list) else None,
                "apply_eligible": False,
                "decision": decision,
                "gold_protected": gold_hit,
                "rationale": (
                    "exact-dup gate-ledger cluster; slice-1 reports only; "
                    "no multi-thought retract without distinct thought_ids sharing fingerprint"
                ),
            }
        )
    return actions


def main() -> int:
    ap = argparse.ArgumentParser(
        description="kengram corpus-dedup plan (D3 slice-1 exact-duplicate, plan-only)"
    )
    ap.add_argument(
        "--gate-clusters",
        type=Path,
        required=True,
        help="JSONL of exact-dup clusters from gate ledger measurement",
    )
    ap.add_argument(
        "--gold-corpus",
        type=Path,
        default=None,
        help="institutional gold-100 jsonl (252 items) for protection set",
    )
    ap.add_argument("--out-dir", type=Path, required=True)
    ap.add_argument("--min-events", type=int, default=2)
    ap.add_argument(
        "--require-gold",
        action="store_true",
        help="fail if gold corpus missing or item count != 252",
    )
    ap.add_argument(
        "--apply",
        action="store_true",
        help="FORBIDDEN on slice-1 plan CLI — always refused",
    )
    args = ap.parse_args()

    if args.apply:
        print("FATAL: slice-1 plan CLI is plan-only; --apply is refused", file=sys.stderr)
        return 2

    args.out_dir.mkdir(parents=True, exist_ok=True)

    gold = load_gold_protection(args.gold_corpus)
    if args.require_gold:
        if gold.get("gold_items_total", 0) != 252:
            print(
                f"FATAL: require-gold expects 252 items, got {gold.get('gold_items_total')}",
                file=sys.stderr,
            )
            return 2

    protected = set(gold.get("protected_thought_ids") or [])
    clusters = load_jsonl(args.gate_clusters)
    actions = plan_from_gate_ledger(clusters, protected, args.min_events)

    gold_blocked = sum(1 for a in actions if a.get("gold_protected"))
    reportable = sum(1 for a in actions if not a.get("gold_protected"))

    plan = {
        "schema": "kengram.corpus_dedup.plan.exact_dup_slice1.v1",
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "spec": "kengram-corpus-hygiene-spec-rev3-2026-07-18.md Part 3 (slice-1 exact-dup allowlist)",
        "mode": "plan_only",
        "apply": False,
        "allowlist": {
            "class": "EXACT_DUPLICATE_ONLY",
            "gate_ledger_clusters": str(args.gate_clusters),
            "gate_clusters_sha256": sha256_file(args.gate_clusters),
            "min_events": args.min_events,
            "gold_passage_clusters_expected": 29,
            "gold_items_expected": 252,
        },
        "gold_protection": gold,
        "counts": {
            "clusters_in": len(clusters),
            "actions": len(actions),
            "gold_blocked": gold_blocked,
            "reportable_non_gold": reportable,
            "apply_eligible": 0,
        },
        "pass_at_10": {
            "protocol": "measure before/after on any apply-eligible plan; slice-1 has zero apply_eligible",
            "baseline_status": "not_run_this_invocation",
            "after_status": "n/a_plan_only_zero_apply",
            "gold_items_untouchable": gold.get("gold_items_total", 0),
        },
        "actions": actions,
    }

    plan_path = args.out_dir / "plan.json"
    manifest_path = args.out_dir / "plan.manifest.jsonl"
    summary_path = args.out_dir / "plan.summary.md"

    plan_bytes = json.dumps(plan, indent=2, sort_keys=True).encode("utf-8")
    plan_path.write_bytes(plan_bytes)
    plan_sha = sha256_bytes(plan_bytes)

    with manifest_path.open("w") as f:
        for a in actions:
            f.write(json.dumps(a, sort_keys=True) + "\n")

    summary = f"""# D3 slice-1 exact-dup corpus-dedup PLAN (plan-only)

- generated_at_utc: {plan["generated_at_utc"]}
- plan_sha256: `{plan_sha}`
- mode: **plan_only** (apply refused)
- allowlist: EXACT_DUPLICATE clusters only
- gate clusters in: {len(clusters)}
- actions: {len(actions)} (apply_eligible=0)
- gold-protected blocked: {gold_blocked}
- gold items: {gold.get("gold_items_total")} (untouchable)
- gold passage exact-dup clusters recorded: {gold.get("passage_exact_dup_clusters")}

## pass@10
Slice-1 produces **zero** apply-eligible rows, so after-plan pass@10 equals baseline
by construction. Baseline measurement remains an operator step against gold-100
before any future apply-eligible slice.

## Next
Jones adversaries this plan artifact. No apply until Knox+adversary co-sign and
Part-3 preconditions (bad-edge remediation, etc.) are met.
"""
    summary_path.write_text(summary)

    meta = {
        "plan_path": str(plan_path),
        "plan_sha256": plan_sha,
        "manifest_path": str(manifest_path),
        "summary_path": str(summary_path),
        "counts": plan["counts"],
    }
    (args.out_dir / "plan.meta.json").write_text(json.dumps(meta, indent=2) + "\n")
    print(json.dumps(meta, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
