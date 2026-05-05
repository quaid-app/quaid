#!/usr/bin/env python3
"""
DAB §8 Conversation Memory benchmark surface for Quaid.

This wrapper runs the LoCoMo and LongMemEval adapters through the real
conversation-memory path:

memory_add_turn -> memory_close_session -> extraction worker -> fact-page query

Regression semantics:
- official gate: no subsection may regress more than 3.0 percentage points
- gate only applies to full, representative-hardware runs
- limited hosted-runner runs are explicitly informational
"""

from __future__ import annotations

import argparse
import json
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import locomo_eval
import longmemeval_adapter

REPO_ROOT = Path(__file__).parent.parent
BASELINE_PATH = REPO_ROOT / "benchmarks" / "baselines" / "conversation_memory.json"


def load_baseline(path: Path) -> dict[str, Any]:
    if not path.exists():
        sys.exit(f"Baseline file not found: {path}")
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def compare_against_baseline(
    score_pct: float,
    baseline_entry: dict[str, Any],
    threshold_points: float,
) -> dict[str, Any]:
    baseline_pct = baseline_entry.get("score_pct")
    if baseline_pct is None:
        return {
            "status": "pending-baseline",
            "passed": None,
            "baseline_score_pct": None,
            "delta_points": None,
            "threshold_points": threshold_points,
        }
    delta_points = score_pct - float(baseline_pct)
    return {
        "status": "pass" if delta_points >= -threshold_points else "fail",
        "passed": delta_points >= -threshold_points,
        "baseline_score_pct": float(baseline_pct),
        "delta_points": delta_points,
        "threshold_points": threshold_points,
    }


def section_average(results: list[dict[str, Any]]) -> float:
    if not results:
        return 0.0
    return sum(result["score_pct"] for result in results) / len(results)


def main() -> None:
    parser = argparse.ArgumentParser(description="DAB §8 Conversation Memory benchmark harness")
    parser.add_argument(
        "--dataset",
        choices=["all", "locomo", "longmemeval"],
        default="all",
        help="Which §8 subsections to run",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=None,
        help="Question limit per subsection. Limited runs are informational and do not apply the regression gate.",
    )
    parser.add_argument(
        "--baseline-file",
        default=str(BASELINE_PATH),
        help="Baseline JSON used for the version-over-version regression gate",
    )
    parser.add_argument(
        "--model-alias",
        default=None,
        help="Override extraction.model_alias for scratch benchmark databases",
    )
    parser.add_argument(
        "--raw-limit",
        type=int,
        default=20,
        help="Initial retrieval depth before filtering down to fact pages",
    )
    parser.add_argument(
        "--wait-timeout",
        type=int,
        default=900,
        help="Seconds to wait for extraction to finish per subsection",
    )
    parser.add_argument("--json", action="store_true", help="Emit machine-readable JSON")
    args = parser.parse_args()

    baseline = load_baseline(Path(args.baseline_file))
    threshold_points = float(baseline.get("regression_threshold_points", 3.0))
    limited_run = args.limit is not None

    subsection_results: list[dict[str, Any]] = []

    with tempfile.TemporaryDirectory(prefix="quaid-dab-section8-") as temp_dir:
        root = Path(temp_dir)

        if args.dataset in {"all", "locomo"}:
            locomo_db = root / "locomo.db"
            locomo_workspace = root / "locomo"
            locomo_sessions = locomo_eval.load_locomo_data()
            locomo_result = locomo_eval.run_conversation_memory_evaluation(
                locomo_sessions,
                db_path=str(locomo_db),
                workspace_dir=locomo_workspace,
                limit=args.limit,
                model_alias=args.model_alias,
                raw_limit=args.raw_limit,
                wait_timeout=args.wait_timeout,
            )
            locomo_result["subsection"] = "LoCoMo"
            if limited_run:
                locomo_result["gate"] = {
                    "status": "informational-limited-run",
                    "passed": None,
                    "baseline_score_pct": None,
                    "delta_points": None,
                    "threshold_points": threshold_points,
                }
            else:
                locomo_result["gate"] = compare_against_baseline(
                    locomo_result["score_pct"],
                    baseline["baselines"]["locomo"],
                    threshold_points,
                )
            subsection_results.append(locomo_result)

        if args.dataset in {"all", "longmemeval"}:
            longmem_db = root / "longmemeval.db"
            longmem_workspace = root / "longmemeval"
            longmem_sessions = longmemeval_adapter.load_longmemeval_sessions("test")
            longmem_result = longmemeval_adapter.run_conversation_memory_evaluation(
                longmem_sessions,
                db_path=str(longmem_db),
                workspace_dir=longmem_workspace,
                limit=args.limit,
                model_alias=args.model_alias,
                raw_limit=args.raw_limit,
                wait_timeout=args.wait_timeout,
            )
            longmem_result["subsection"] = "LongMemEval"
            if limited_run:
                longmem_result["gate"] = {
                    "status": "informational-limited-run",
                    "passed": None,
                    "baseline_score_pct": None,
                    "delta_points": None,
                    "threshold_points": threshold_points,
                }
            else:
                longmem_result["gate"] = compare_against_baseline(
                    longmem_result["score_pct"],
                    baseline["baselines"]["longmemeval"],
                    threshold_points,
                )
            subsection_results.append(longmem_result)

    failing_subsections = [
        result["subsection"]
        for result in subsection_results
        if result["gate"]["passed"] is False
    ]
    pending_baselines = [
        result["subsection"]
        for result in subsection_results
        if result["gate"]["status"] == "pending-baseline"
    ]

    if limited_run:
        gate_status = "informational-limited-run"
        gate_passed = None
    elif failing_subsections:
        gate_status = "fail"
        gate_passed = False
    elif pending_baselines:
        gate_status = "pending-baseline"
        gate_passed = None
    else:
        gate_status = "pass"
        gate_passed = True

    report = {
        "section": "DAB §8 Conversation Memory",
        "generated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "baseline_file": str(Path(args.baseline_file)),
        "regression_threshold_points": threshold_points,
        "limited_run": limited_run,
        "gate_status": gate_status,
        "gate_passed": gate_passed,
        "section_score_pct": section_average(subsection_results),
        "subsections": subsection_results,
        "truth_boundary": (
            "Regression gating only applies to full representative-hardware runs. "
            "Limited runs and the hosted-runner workflow are informational smoke hooks."
        ),
    }

    if args.json:
        print(json.dumps(report, indent=2))
    else:
        print("=== DAB §8 Conversation Memory ===")
        print(f"Threshold: no regression > {threshold_points:.1f} points")
        if limited_run:
            print("Run type: informational limited run (gate skipped)")
        for result in subsection_results:
            print(f"\n[{result['subsection']}]")
            print(f"Score:      {result['score_pct']:.2f}%")
            print(f"Metric:     {result['metric']}")
            print(f"Fact pages: {result['fact_page_count']}")
            gate = result["gate"]
            if gate["status"] == "informational-limited-run":
                print("Gate:       informational limited run")
            elif gate["status"] == "pending-baseline":
                print("Gate:       pending baseline")
            else:
                status = "PASS" if gate["passed"] else "FAIL"
                print(
                    f"Gate:       {status} "
                    f"(baseline {gate['baseline_score_pct']:.2f}%, "
                    f"delta {gate['delta_points']:+.2f} points)"
                )
        print(f"\nSection score: {report['section_score_pct']:.2f}%")
        print(f"Gate status:   {gate_status}")

    if gate_passed is False:
        sys.exit(1)


if __name__ == "__main__":
    main()
