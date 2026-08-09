#!/usr/bin/env python3
"""Delivery A frozen-six mutant ceremony — smith section-10 bar (PR20 ceremony repair).

Transform definitions:
  M1 DELETE query-embedding recorder call (not enum substitution)
  M2 FTS timeout → healthy empty (early return in storage_leg_fail_open for ThoughtFts)
  M3 BYPASS/DELETE rerank recorder call (not enum substitution)
  M4 remove /health mount
  M5 remove degradations from MCP serialization
  M6 shift per-leg counter index to neighbor

Checker rules (receiver-auditable from raw logs alone):
  - exactly one preimage for each transform (count == 1)
  - mutant sha ≠ baseline sha
  - RED only when exit≠0 AND raw output contains capability_after_entry marker
    AND the mutant-unique KENGRAM_DELIVERY_A_RED terminal marker
  - reject compile/setup/import-only failures and running-0-tests
  - exact-hash restore after each mutant before next
  - GREEN after restore (exit 0)
  - RECEIPT_OK only if every check above passes (never unconditional)
"""
from __future__ import annotations

import hashlib
import json
import os
import re
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
    return subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=WT, text=True
    ).strip()


def count_preimages(haystack: str, needle: str) -> int:
    if not needle:
        return 0
    return haystack.count(needle)


def is_compile_or_setup_failure(stdout: str, stderr: str) -> bool:
    combined = (stdout or "") + "\n" + (stderr or "")
    if "running 0 tests" in combined:
        return True
    if "could not compile" in combined and "panicked at" not in combined:
        return True
    if re.search(r"error\[E\d+", combined) and "panicked at" not in combined:
        # pure compile failure without test panic
        if "assertion" not in combined.lower() and "FAILED" not in combined:
            return True
    if "error: test failed" not in combined and "FAILED" not in combined:
        if "error:" in combined and "panicked at" not in combined:
            return True
    return False


def ceremony(
    name: str,
    file_rel: str,
    transform_desc: str,
    preimage: str,
    mutant_body: str,
    cargo_cmd: list[str],
    capability_after_entry: str,
    terminal_marker: str,
) -> dict:
    path = WT / file_rel
    baseline = path.read_bytes()
    baseline_sha = sha256_bytes(baseline)
    text = baseline.decode("utf-8")

    n = count_preimages(text, preimage)
    if n != 1:
        raise RuntimeError(
            f"{name}: expected exactly-one-preimage, found {n} for transform"
        )

    mut = text.replace(preimage, mutant_body, 1)
    if mut == text:
        raise RuntimeError(f"{name}: transform no-op after replace")
    if count_preimages(mut, preimage) != 0:
        raise RuntimeError(f"{name}: preimage still present after transform")

    path.write_text(mut)
    mutant_sha = sha256_file(path)
    if mutant_sha == baseline_sha:
        path.write_bytes(baseline)
        raise RuntimeError(f"{name}: mutant sha == baseline")

    red = run(cargo_cmd)
    red_combined = (red.stdout or "") + "\n" + (red.stderr or "")
    red_out = (
        f"$ {' '.join(cargo_cmd)}\n"
        f"exit={red.returncode}\n\n"
        f"--- stdout ---\n{red.stdout}\n\n"
        f"--- stderr ---\n{red.stderr}\n"
    )
    red_path = OUT_DIR / f"{name}-RED.log"
    red_path.write_text(red_out)

    # Always restore before raising so tree stays clean on failure
    def restore() -> str:
        path.write_bytes(baseline)
        got = sha256_file(path)
        if got != baseline_sha:
            raise RuntimeError(f"{name}: restore mismatch {got} != {baseline_sha}")
        return got

    receipt_ok = True
    fail_reasons: list[str] = []

    if red.returncode == 0:
        receipt_ok = False
        fail_reasons.append("expected RED (exit!=0) got GREEN")
    else:
        if is_compile_or_setup_failure(red.stdout or "", red.stderr or ""):
            receipt_ok = False
            fail_reasons.append("compile/setup/import failure rejected as zero evidence")
        if capability_after_entry not in red_combined:
            receipt_ok = False
            fail_reasons.append(
                f"missing capability-after-entry marker: {capability_after_entry!r}"
            )
        if terminal_marker not in red_combined:
            receipt_ok = False
            fail_reasons.append(
                f"missing unique terminal RED marker: {terminal_marker!r}"
            )
        # require a real test failure (panic/assert/FAILED), not bare nonzero
        if (
            "panicked at" not in red_combined
            and "assertion" not in red_combined.lower()
            and "FAILED" not in red_combined
            and "failures:" not in red_combined
        ):
            receipt_ok = False
            fail_reasons.append("RED without assertion/panic/FAILED evidence")

    restore_sha = restore()

    green = run(cargo_cmd)
    green_combined = (green.stdout or "") + "\n" + (green.stderr or "")
    green_out = (
        f"$ {' '.join(cargo_cmd)}\n"
        f"exit={green.returncode}\n\n"
        f"--- stdout ---\n{green.stdout}\n\n"
        f"--- stderr ---\n{green.stderr}\n"
    )
    green_path = OUT_DIR / f"{name}-GREEN.log"
    green_path.write_text(green_out)
    if green.returncode != 0:
        receipt_ok = False
        fail_reasons.append("expected GREEN after restore")

    if not receipt_ok:
        raise RuntimeError(
            f"{name}: RECEIPT_REJECTED: {'; '.join(fail_reasons)}\n"
            f"RED log tail:\n{red_out[-2500:]}"
        )

    return {
        "name": name,
        "file": file_rel,
        "transform": transform_desc,
        "preimage_count": 1,
        "command": " ".join(cargo_cmd),
        "capability_after_entry": capability_after_entry,
        "terminal_marker": terminal_marker,
        "baseline_sha256": baseline_sha,
        "mutant_sha256": mutant_sha,
        "restore_sha256": restore_sha,
        "red_exit": red.returncode,
        "red_log": str(red_path.relative_to(WT)),
        "red_contains_capability_marker": True,
        "red_contains_terminal_marker": True,
        "green_exit": green.returncode,
        "green_log": str(green_path.relative_to(WT)),
        "RECEIPT_OK": True,
    }


def main() -> int:
    head = git_head()
    results: list[dict] = []

    # --- M1: DELETE query-embedding recorder call ---
    m1_pre = """            record_degradation(
                counters.as_ref(),
                &mut degradations,
                SearchDegradation::new(
                    SearchLeg::QueryEmbedding,
                    reason,
                    DegradationFallback::LexicalOnly,
                    timeout_ms,
                    1,
                ),
                search_seq,
            );
"""
    m1_mut = """            // MUTANT_M1: query-embedding recorder call deleted
            let _ = (counters.as_ref(), &mut degradations, reason, timeout_ms, search_seq);
"""
    results.append(
        ceremony(
            name="M1_query_embedding_recorder_deletion",
            file_rel="crates/kengram-mcp/src/search.rs",
            transform_desc="DELETE query-embedding recorder call (SearchLeg::QueryEmbedding record_degradation)",
            preimage=m1_pre,
            mutant_body=m1_mut,
            cargo_cmd=[
                "cargo",
                "test",
                "-p",
                "kengram-mcp",
                "--lib",
                "search::tests::search_thoughts_degrades_when_embedder_fails",
                "--",
                "--exact",
                "--test-threads=1",
                "--nocapture",
            ],
            # capability after entry: embedder-fail path + test name
            capability_after_entry="search_thoughts_degrades_when_embedder_fails",
            terminal_marker="KENGRAM_DELIVERY_A_RED:V1_query_embedding_receipt",
        )
    )
    print("OK", results[-1]["name"], flush=True)

    # --- M2: FTS timeout → healthy empty ---
    m2_pre = """fn storage_leg_fail_open(
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
    m2_mut = """fn storage_leg_fail_open(
    e: &kengram_storage::StorageError,
    leg: SearchLeg,
    lexical_timeout_ms: u64,
    counters: Option<&std::sync::Arc<SearchCounters>>,
    degradations: &mut Vec<SearchDegradation>,
    search_seq: u64,
    failed_attempts: u32,
) {
    // MUTANT_M2: convert production FTS timeout outcome to healthy empty
    if leg == SearchLeg::ThoughtFts {
        let _ = (e, lexical_timeout_ms, counters, degradations, search_seq, failed_attempts);
        return;
    }
    let reason = if e.is_query_canceled() {
        DegradationReason::Timeout
    } else {
        DegradationReason::Storage
    };"""
    results.append(
        ceremony(
            name="M2_fts_timeout_healthy_empty",
            file_rel="crates/kengram-mcp/src/search.rs",
            transform_desc="ThoughtFts timeout fail-open becomes healthy empty (no record)",
            preimage=m2_pre,
            mutant_body=m2_mut,
            cargo_cmd=[
                "cargo",
                "test",
                "-p",
                "kengram-mcp",
                "--lib",
                "search::tests::search_thoughts_fts_timeout_causally_armed",
                "--",
                "--exact",
                "--test-threads=1",
                "--nocapture",
            ],
            capability_after_entry="search_thoughts_fts_timeout_causally_armed",
            terminal_marker="KENGRAM_DELIVERY_A_RED:V2_thought_fts_timeout_receipt",
        )
    )
    print("OK", results[-1]["name"], flush=True)

    # --- M3: BYPASS/DELETE rerank recorder ---
    m3_pre = """            record_degradation(
                counters,
                degradations,
                SearchDegradation::new(
                    SearchLeg::Rerank,
                    reason_from_reranker(&e),
                    DegradationFallback::RrfRecency,
                    rerank_timeout_ms,
                    1,
                ),
                search_seq,
            );
            return false;
"""
    m3_mut = """            // MUTANT_M3: rerank timeout branch returns false without recording
            let _ = (counters, degradations, &e, rerank_timeout_ms, search_seq);
            return false;
"""
    results.append(
        ceremony(
            name="M3_rerank_recorder_deletion",
            file_rel="crates/kengram-mcp/src/search.rs",
            transform_desc="DELETE/BYPASS rerank recorder call on timeout branch (return false without record)",
            preimage=m3_pre,
            mutant_body=m3_mut,
            cargo_cmd=[
                "cargo",
                "test",
                "-p",
                "kengram-mcp",
                "--lib",
                "search::tests::search_thoughts_rerank_timeout_via_tei_wiremock",
                "--",
                "--exact",
                "--test-threads=1",
                "--nocapture",
            ],
            capability_after_entry="search_thoughts_rerank_timeout_via_tei_wiremock",
            terminal_marker="KENGRAM_DELIVERY_A_RED:V3_rerank_timeout_receipt",
        )
    )
    print("OK", results[-1]["name"], flush=True)

    # --- M4: remove /health mount ---
    m4_pre = """pub fn mount_health(router: axum::Router<HealthState>) -> axum::Router<HealthState> {
    router.route("/health", axum::routing::get(health_handler))
}"""
    m4_mut = """pub fn mount_health(router: axum::Router<HealthState>) -> axum::Router<HealthState> {
    // MUTANT_M4: /health route mount removed
    router
}"""
    results.append(
        ceremony(
            name="M4_health_mount_removal",
            file_rel="crates/kengram-cli/src/health.rs",
            transform_desc="Remove GET /health route mount",
            preimage=m4_pre,
            mutant_body=m4_mut,
            cargo_cmd=[
                "cargo",
                "test",
                "-p",
                "kengram-cli",
                "--bin",
                "kengram",
                "health::tests::health_route_via_axum_router",
                "--",
                "--exact",
                "--test-threads=1",
                "--nocapture",
            ],
            capability_after_entry="health_route_via_axum_router",
            terminal_marker="KENGRAM_DELIVERY_A_RED:V4_health_route_mount",
        )
    )
    print("OK", results[-1]["name"], flush=True)

    # --- M5: remove degradations from MCP serialization ---
    m5_pre = '"degradations": resp.degradations,'
    m5_mut = '"degradations_removed": [], // MUTANT_M5'
    results.append(
        ceremony(
            name="M5_mcp_degradations_serialization",
            file_rel="crates/kengram-mcp/src/server.rs",
            transform_desc="Remove degradations from MCP JSON serialization",
            preimage=m5_pre,
            mutant_body=m5_mut,
            cargo_cmd=[
                "cargo",
                "test",
                "-p",
                "kengram-mcp",
                "--lib",
                "server::tests::search_thoughts_tool_emits_degradations_json",
                "--",
                "--exact",
                "--test-threads=1",
                "--nocapture",
            ],
            capability_after_entry="search_thoughts_tool_emits_degradations_json",
            terminal_marker="KENGRAM_DELIVERY_A_RED:V4_mcp_degradations_serialization",
        )
    )
    print("OK", results[-1]["name"], flush=True)

    # --- M6: neighbor counter index ---
    m6_pre = "        let li = d.leg.index();"
    m6_mut = "        let li = (d.leg.index() + 1) % SearchLeg::ALL.len(); // MUTANT_M6"
    results.append(
        ceremony(
            name="M6_neighbor_counter_index_shift",
            file_rel="crates/kengram-mcp/src/degradation.rs",
            transform_desc="Per-leg counter index shifted to neighboring enum member",
            preimage=m6_pre,
            mutant_body=m6_mut,
            cargo_cmd=[
                "cargo",
                "test",
                "-p",
                "kengram-mcp",
                "--lib",
                "degradation::tests::record_increments_exact_cell",
                "--",
                "--exact",
                "--test-threads=1",
                "--nocapture",
            ],
            capability_after_entry="record_increments_exact_cell",
            terminal_marker="KENGRAM_DELIVERY_A_RED:M6_counter_index",
        )
    )
    print("OK", results[-1]["name"], flush=True)

    all_ok = all(r.get("RECEIPT_OK") is True for r in results)
    # final baseline rehash of all touched files
    final_hashes = {
        "search.rs": sha256_file(WT / "crates/kengram-mcp/src/search.rs"),
        "degradation.rs": sha256_file(WT / "crates/kengram-mcp/src/degradation.rs"),
        "server.rs": sha256_file(WT / "crates/kengram-mcp/src/server.rs"),
        "health.rs": sha256_file(WT / "crates/kengram-cli/src/health.rs"),
    }

    summary = {
        "kind": "kengram-pr20-delivery-a-frozen-six-ceremony",
        "spec_section": "10",
        "executed_code_head": head,
        "certifier": "diesel",
        "when": datetime.now(timezone.utc).isoformat(),
        "transforms": {
            "M1": "DELETE query-embedding recorder call",
            "M2": "FTS timeout to healthy empty",
            "M3": "BYPASS/DELETE rerank recorder call",
            "M4": "/health mount removal",
            "M5": "MCP degradations serialization removal",
            "M6": "neighbor counter index shift",
        },
        "checker_rules": [
            "exactly-one-preimage",
            "RED requires capability_after_entry AND unique KENGRAM_DELIVERY_A_RED terminal marker",
            "reject compile/setup/import/running-0-tests",
            "exact-hash restore between mutants",
            "GREEN after restore",
            "RECEIPT_OK only when all checks pass (never unconditional)",
        ],
        "results": results,
        "final_file_sha256": final_hashes,
        "ALL_SIX_FROZEN_MUTANT_RECEIPTS_OK": all_ok,
    }

    md_lines = [
        "# KENGRAM Delivery A — frozen six mutant receipts (diesel ceremony repair)",
        "",
        f"- executed_code_head: `{head}`",
        f"- certifier: `diesel`",
        f"- when: `{summary['when']}`",
        f"- frozen_spec_section: `10`",
        "",
        "Checker: exactly-one-preimage; RED only with capability-after-entry + unique",
        "`KENGRAM_DELIVERY_A_RED` terminal marker; reject compile/setup; exact restore;",
        "GREEN; `RECEIPT_OK` never unconditional.",
        "",
    ]
    for r in results:
        md_lines += [
            f"## Mutant {r['name']}",
            "",
            f"- file: `{r['file']}`",
            f"- transform: {r['transform']}",
            f"- preimage_count: `{r['preimage_count']}`",
            f"- command: `{r['command']}`",
            f"- capability_after_entry: `{r['capability_after_entry']}`",
            f"- terminal_marker: `{r['terminal_marker']}`",
            f"- baseline_sha256: `{r['baseline_sha256']}`",
            f"- mutant_sha256: `{r['mutant_sha256']}`",
            f"- RED_log: `{r['red_log']}` exit={r['red_exit']}",
            f"- restore_sha256: `{r['restore_sha256']}` MATCH",
            f"- GREEN_log: `{r['green_log']}` exit={r['green_exit']}",
            f"- **RECEIPT_OK** `{r['name']}` = `{r['RECEIPT_OK']}`",
            "",
        ]
    md_lines += [
        "## Summary",
        "",
        f"**ALL_SIX_FROZEN_MUTANT_RECEIPTS_OK** = `{all_ok}` executed_code_head=`{head}`",
        "",
        "Final file hashes after ceremony:",
        "",
    ]
    for k, v in final_hashes.items():
        md_lines.append(f"- `{k}`: `{v}`")
    md_lines.append("")

    (WT / "receipts" / "RECEIPT-delivery-a-frozen-six-mutants.md").write_text(
        "\n".join(md_lines)
    )
    (OUT_DIR / "INDEX.md").write_text("\n".join(md_lines))
    (WT / "receipts" / "RECEIPT-delivery-a-frozen-six-mutants.json").write_text(
        json.dumps(summary, indent=2) + "\n"
    )
    print(json.dumps({"ALL_SIX": all_ok, "head": head}, indent=2))
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
