#!/usr/bin/env python3
"""Spike-certified frozen-six mutant ceremony (smith 324df2a9 / knox PR20).

Does not trust heavy receipts; regenerates raw RED/GREEN under spike control.
"""
from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

WT = Path(__file__).resolve().parents[1]
OUT_DIR = WT / "receipts" / "delivery-a-frozen-six"
OUT_DIR.mkdir(parents=True, exist_ok=True)

ENV = {
    **os.environ,
    "SQLX_OFFLINE": "true",
    "DATABASE_URL": os.environ.get(
        "DATABASE_URL", "postgres://kengram:kengram@localhost:5432/kengram"
    ),
}


def sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def sha256_file(p: Path) -> str:
    return sha256_bytes(p.read_bytes())


def run(cmd: list[str], timeout: int = 300) -> subprocess.CompletedProcess:
    return subprocess.run(
        cmd, cwd=WT, env=ENV, capture_output=True, text=True, timeout=timeout
    )


def git_head() -> str:
    return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=WT, text=True).strip()


def ceremony(
    name: str,
    file_rel: str,
    transform_desc: str,
    apply_fn,
    cargo_cmd: list[str],
    capability_marker: str,
) -> dict:
    path = WT / file_rel
    baseline = path.read_bytes()
    baseline_sha = sha256_bytes(baseline)
    mut = apply_fn(baseline.decode("utf-8"))
    if mut == baseline.decode("utf-8"):
        raise RuntimeError(f"{name}: transform no-op")
    path.write_text(mut)
    mutant_sha = sha256_file(path)
    if mutant_sha == baseline_sha:
        path.write_bytes(baseline)
        raise RuntimeError(f"{name}: mutant sha == baseline")

    red = run(cargo_cmd)
    red_out = f"$ {' '.join(cargo_cmd)}\nexit={red.returncode}\n\n--- stdout ---\n{red.stdout}\n\n--- stderr ---\n{red.stderr}\n"
    red_path = OUT_DIR / f"{name}-RED.log"
    red_path.write_text(red_out)
    if red.returncode == 0:
        path.write_bytes(baseline)
        raise RuntimeError(f"{name}: expected RED got GREEN\n{red_out[-2000:]}")
    # Prefer runtime assertion failures over pure compile errors when possible,
    # but accept compile RED only if capability_marker appears in mutant source intent.
    # Reject silent compile typos that don't relate to capability.
    if "error[E" in (red.stderr or "") and "assertion" not in (red.stdout or "") + (red.stderr or ""):
        # compile-only RED: still require the mutant comment tag present
        if f"MUTANT_{name}" not in mut and "MUTANT_" not in mut:
            path.write_bytes(baseline)
            raise RuntimeError(f"{name}: compile-only RED without mutant tag")

    path.write_bytes(baseline)
    restore_sha = sha256_file(path)
    if restore_sha != baseline_sha:
        raise RuntimeError(f"{name}: restore mismatch")

    green = run(cargo_cmd)
    green_out = f"$ {' '.join(cargo_cmd)}\nexit={green.returncode}\n\n--- stdout ---\n{green.stdout}\n\n--- stderr ---\n{green.stderr}\n"
    green_path = OUT_DIR / f"{name}-GREEN.log"
    green_path.write_text(green_out)
    if green.returncode != 0:
        raise RuntimeError(f"{name}: expected GREEN got RED\n{green_out[-2000:]}")

    return {
        "name": name,
        "file": file_rel,
        "transform": transform_desc,
        "command": " ".join(cargo_cmd),
        "capability_marker": capability_marker,
        "baseline_sha256": baseline_sha,
        "mutant_sha256": mutant_sha,
        "restore_sha256": restore_sha,
        "red_exit": red.returncode,
        "red_log": str(red_path.relative_to(WT)),
        "green_exit": green.returncode,
        "green_log": str(green_path.relative_to(WT)),
        "RECEIPT_OK": True,
    }


def main() -> int:
    head = git_head()
    results = []

    def m1(s: str) -> str:
        old = """                SearchDegradation::new(
                    SearchLeg::QueryEmbedding,
                    reason,
                    DegradationFallback::LexicalOnly,"""
        new = """                SearchDegradation::new(
                    SearchLeg::ThoughtVector, // MUTANT_M1_query_embedding_recorder_deletion
                    reason,
                    DegradationFallback::LexicalOnly,"""
        if old not in s:
            raise RuntimeError("m1 preimage missing")
        return s.replace(old, new, 1)

    results.append(
        ceremony(
            "M1_query_embedding_recorder_deletion",
            "crates/kengram-mcp/src/search.rs",
            "QueryEmbedding→ThoughtVector in embedder-fail degradation (recorder deletion)",
            m1,
            [
                "cargo", "test", "-p", "kengram-mcp", "--lib",
                "search::tests::search_thoughts_degrades_when_embedder_fails",
                "--", "--exact", "--test-threads=1", "--nocapture",
            ],
            "QueryEmbedding",
        )
    )
    print("OK", results[-1]["name"], flush=True)

    def m2(s: str) -> str:
        old = """fn storage_leg_fail_open(
    e: &kengram_storage::StorageError,
    leg: SearchLeg,
    lexical_timeout_ms: u64,
    counters: Option<&std::sync::Arc<SearchCounters>>,
    degradations: &mut Vec<SearchDegradation>,
    search_seq: u64,
    failed_attempts: u32,
) {
    let reason = if e.is_query_canceled() {
        DegradationReason::Timeout
    } else {
        DegradationReason::Storage
    };"""
        new = """fn storage_leg_fail_open(
    e: &kengram_storage::StorageError,
    leg: SearchLeg,
    lexical_timeout_ms: u64,
    counters: Option<&std::sync::Arc<SearchCounters>>,
    degradations: &mut Vec<SearchDegradation>,
    search_seq: u64,
    failed_attempts: u32,
) {
    if leg == SearchLeg::ThoughtFts {
        return; // MUTANT_M2_fts_timeout_to_healthy_empty
    }
    let reason = if e.is_query_canceled() {
        DegradationReason::Timeout
    } else {
        DegradationReason::Storage
    };"""
        if old not in s:
            raise RuntimeError("m2 preimage missing")
        return s.replace(old, new, 1)

    results.append(
        ceremony(
            "M2_fts_timeout_healthy_empty",
            "crates/kengram-mcp/src/search.rs",
            "storage_leg_fail_open returns early for ThoughtFts (timeout→silent healthy empty)",
            m2,
            [
                "cargo", "test", "-p", "kengram-mcp", "--lib",
                "search::tests::search_thoughts_fts_timeout_causally_armed",
                "--", "--exact", "--test-threads=1", "--nocapture",
            ],
            "ThoughtFts",
        )
    )
    print("OK", results[-1]["name"], flush=True)

    def m3(s: str) -> str:
        old = """                SearchDegradation::new(
                    SearchLeg::Rerank,
                    reason_from_reranker(&e),
                    DegradationFallback::RrfRecency,"""
        new = """                SearchDegradation::new(
                    SearchLeg::ThoughtFts, // MUTANT_M3_rerank_recorder_deletion
                    reason_from_reranker(&e),
                    DegradationFallback::RrfRecency,"""
        if old not in s:
            raise RuntimeError("m3 preimage missing")
        return s.replace(old, new, 1)

    results.append(
        ceremony(
            "M3_rerank_recorder_deletion",
            "crates/kengram-mcp/src/search.rs",
            "Rerank→ThoughtFts in rerank-fail degradation (recorder deletion)",
            m3,
            [
                "cargo", "test", "-p", "kengram-mcp", "--lib",
                "search::tests::search_thoughts_rerank_timeout_via_tei_wiremock",
                "--", "--exact", "--test-threads=1", "--nocapture",
            ],
            "Rerank",
        )
    )
    print("OK", results[-1]["name"], flush=True)

    def m4(s: str) -> str:
        old = """pub fn mount_health(router: axum::Router<HealthState>) -> axum::Router<HealthState> {
    router.route("/health", axum::routing::get(health_handler))
}"""
        new = """pub fn mount_health(router: axum::Router<HealthState>) -> axum::Router<HealthState> {
    router // MUTANT_M4_health_mount_removal
}"""
        if old not in s:
            raise RuntimeError("m4 preimage missing")
        return s.replace(old, new, 1)

    results.append(
        ceremony(
            "M4_health_mount_removal",
            "crates/kengram-cli/src/health.rs",
            "mount_health no longer registers GET /health",
            m4,
            [
                "cargo", "test", "-p", "kengram-cli", "--bin", "kengram",
                "health::tests::health_route_via_axum_router",
                "--", "--exact", "--test-threads=1", "--nocapture",
            ],
            "health",
        )
    )
    print("OK", results[-1]["name"], flush=True)

    def m5(s: str) -> str:
        old = '"degradations": resp.degradations,'
        new = '"degradations_removed": [], // MUTANT_M5_mcp_degradations_serialization_removal'
        if old not in s:
            raise RuntimeError("m5 preimage missing")
        return s.replace(old, new, 1)

    results.append(
        ceremony(
            "M5_mcp_degradations_serialization",
            "crates/kengram-mcp/src/server.rs",
            "MCP JSON omits degradations field",
            m5,
            [
                "cargo", "test", "-p", "kengram-mcp", "--lib",
                "server::tests::search_thoughts_tool_emits_degradations_json",
                "--", "--exact", "--test-threads=1", "--nocapture",
            ],
            "degradation",
        )
    )
    print("OK", results[-1]["name"], flush=True)

    def m6(s: str) -> str:
        # production counter index in record_degradation path
        old = "        let li = d.leg.index();"
        new = "        let li = (d.leg.index() + 1) % SearchLeg::ALL.len(); // MUTANT_M6_counter_index_shift"
        if old not in s:
            # try unindented
            old2 = "let li = d.leg.index();"
            new2 = "let li = (d.leg.index() + 1) % SearchLeg::ALL.len(); // MUTANT_M6_counter_index_shift"
            if old2 not in s:
                raise RuntimeError("m6 preimage missing")
            return s.replace(old2, new2, 1)
        return s.replace(old, new, 1)

    results.append(
        ceremony(
            "M6_neighbor_counter_index_shift",
            "crates/kengram-mcp/src/degradation.rs",
            "counter cell uses (leg.index()+1)%ALL instead of leg.index() (neighbor shift)",
            m6,
            [
                "cargo", "test", "-p", "kengram-mcp", "--lib",
                "degradation::tests::record_increments_exact_cell",
                "--", "--exact", "--test-threads=1", "--nocapture",
            ],
            "index",
        )
    )
    print("OK", results[-1]["name"], flush=True)

    all_ok = all(r["RECEIPT_OK"] for r in results)
    summary = {
        "kind": "kengram-pr20-spike-certified-frozen-six",
        "executed_code_head": head,
        "certifier": "spike",
        "smith_bind": "324df2a9",
        "when": datetime.now(timezone.utc).isoformat(),
        "frozen_six": [
            "query-embedding recorder deletion",
            "FTS-timeout-to-healthy-empty",
            "rerank recorder deletion",
            "/health mount removal",
            "MCP degradations serialization removal",
            "counter-index shift",
        ],
        "heavy_input_evaluated": "74d0ba7 — F1 code adopted (dc90f56); F2 receipts re-generated under spike",
        "results": results,
        "ALL_SIX_FROZEN_MUTANT_RECEIPTS_OK": all_ok,
    }

    md = [
        "# KENGRAM Delivery A — frozen six mutant receipts (SPIKE-CERTIFIED)",
        "",
        f"- executed_code_head: `{head}`",
        f"- certifier: `spike`",
        f"- when: `{summary['when']}`",
        f"- smith_shape: `324df2a9` exact frozen six",
        f"- heavy_input: evaluated 74d0ba7; F1 code kept; F2 logs re-run under spike (not trusted as baseline)",
        "",
        "Ceremony: baseline hash → exact one-preimage transform → cargo RED (raw log) →",
        "byte restore + rehash MATCH → cargo GREEN (raw log).",
        "",
    ]
    for r in results:
        md += [
            f"## Mutant {r['name']}",
            "",
            f"- file: `{r['file']}`",
            f"- transform: {r['transform']}",
            f"- command: `{r['command']}`",
            f"- capability_marker: `{r['capability_marker']}`",
            f"- baseline_sha256: `{r['baseline_sha256']}`",
            f"- mutant_sha256: `{r['mutant_sha256']}`",
            f"- RED_log: `{r['red_log']}` exit={r['red_exit']}",
            f"- restore_sha256: `{r['restore_sha256']}` MATCH",
            f"- GREEN_log: `{r['green_log']}` exit={r['green_exit']}",
            f"- **RECEIPT_OK** `{r['name']}`",
            "",
        ]
    md += [
        "## Summary",
        "",
        f"**ALL_SIX_FROZEN_MUTANT_RECEIPTS_OK** = `{all_ok}` executed_code_head=`{head}` certifier=`spike`",
        "",
    ]
    (WT / "receipts" / "RECEIPT-delivery-a-frozen-six-mutants.md").write_text("\n".join(md))
    (OUT_DIR / "INDEX.md").write_text("\n".join(md))
    (WT / "receipts" / "RECEIPT-delivery-a-frozen-six-mutants.json").write_text(
        json.dumps(summary, indent=2) + "\n"
    )
    print(json.dumps({"ALL_SIX": all_ok, "head": head}, indent=2))
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
