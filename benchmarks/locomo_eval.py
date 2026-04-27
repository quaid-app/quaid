#!/usr/bin/env python3
"""
benchmarks/locomo_eval.py

LoCoMo (Long-term Conversational Memory) evaluation for Quaid.

Imports LoCoMo conversations, evaluates hybrid search F1 against ground truth,
and compares against a FTS5-only baseline.

Target: hybrid search F1 >= +30% over FTS5-only baseline

Prerequisites:
  - Dataset downloaded: ./benchmarks/prep_datasets.sh locomo
  - Quaid binary built: cargo build --release
  - Python deps: pip install -r benchmarks/requirements.txt

Usage:
  python benchmarks/locomo_eval.py
  python benchmarks/locomo_eval.py --baseline-only    # measure FTS5 baseline
  python benchmarks/locomo_eval.py --limit 50         # evaluate first 50 queries
  python benchmarks/locomo_eval.py --json             # JSON output

Environment:
  QUAID_BIN   — path to quaid binary (default: ./target/release/quaid)
  DATASETS_DIR — dataset root (default: ./benchmarks/datasets)
"""

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

import numpy as np
from rouge_score import rouge_scorer
from tqdm import tqdm

# ── Config ────────────────────────────────────────────────────────────────────

REPO_ROOT = Path(__file__).parent.parent
DATASETS_DIR = Path(os.environ.get("DATASETS_DIR", REPO_ROOT / "benchmarks" / "datasets"))
QUAID_BIN = os.environ.get("QUAID_BIN", str(REPO_ROOT / "target" / "release" / "quaid"))
LOCOMO_DIR = DATASETS_DIR / "locomo"

TARGET_DELTA_F1 = 0.30  # +30% F1 over FTS5 baseline

# ── Dataset loader ────────────────────────────────────────────────────────────

def load_locomo_data(data_dir: Path | None = None) -> list[dict[str, Any]]:
    """Load LoCoMo conversations from the downloaded dataset."""
    search_dir = data_dir or LOCOMO_DIR
    if not search_dir.exists():
        sys.exit(
            f"LoCoMo dataset not found at {search_dir}.\n"
            f"Run: ./benchmarks/prep_datasets.sh locomo"
        )

    candidates = (
        list(search_dir.rglob("*.json"))
        + list(search_dir.rglob("*.jsonl"))
    )
    if not candidates:
        sys.exit(f"No LoCoMo JSON files found in {search_dir}.")

    sessions = []
    for path in sorted(candidates)[:10]:  # LoCoMo10 = first 10 conversations
        with open(path) as f:
            text = f.read().strip()
        if text.startswith("["):
            data = json.loads(text)
            if isinstance(data, list):
                sessions.extend(data)
        else:
            for line in text.splitlines():
                if line.strip():
                    sessions.append(json.loads(line))
    return sessions


# ── Importer ──────────────────────────────────────────────────────────────────

def import_conversations(sessions: list[dict], db_path: str) -> int:
    """Import LoCoMo conversations as quaid pages. Returns count."""
    with tempfile.TemporaryDirectory() as tmpdir:
        pages_dir = Path(tmpdir) / "locomo"
        pages_dir.mkdir()

        for session in sessions:
            session_id = session.get("conversation_id", session.get("id", "locomo"))
            slug = f"conversations/{_sanitize_slug(str(session_id))}"

            turns = session.get("conversation", session.get("utterances", []))
            timeline_lines = []
            for turn in turns:
                speaker = turn.get("speaker", turn.get("role", "unknown"))
                text = turn.get("text", turn.get("content", ""))
                ts = turn.get("timestamp", "2024-01")
                timeline_lines.append(f"- **{ts}** | {speaker} — {text[:200]}")

            content = "\n".join([
                "---",
                f"slug: {slug}",
                f"title: Conversation {session_id}",
                "type: conversation",
                "wing: conversations",
                "---",
                f"# Conversation {session_id}",
                "",
                session.get("summary", "Long-term conversational memory session."),
                "---",
                *timeline_lines,
            ])

            page_path = pages_dir / f"{_sanitize_slug(str(session_id))}.md"
            page_path.write_text(content)

        result = subprocess.run(
            [QUAID_BIN, "--db", db_path, "import", str(pages_dir)],
            capture_output=True, text=True
        )
        if result.returncode != 0:
            print(f"Import warning: {result.stderr}", file=sys.stderr)

    return len(sessions)


# ── Retrieval ─────────────────────────────────────────────────────────────────

def run_hybrid_query(query: str, db_path: str, k: int = 5) -> list[str]:
    """Run hybrid search (FTS5 + vector) via quaid query."""
    result = subprocess.run(
        [QUAID_BIN, "--db", db_path, "query", query, "--json"],
        capture_output=True, text=True, timeout=30
    )
    if result.returncode != 0:
        return []
    try:
        data = json.loads(result.stdout)
        items = data if isinstance(data, list) else data.get("results", [])
        return [r.get("compiled_truth", "") + " " + r.get("summary", "") for r in items[:k]]
    except (json.JSONDecodeError, KeyError):
        return []


def run_fts5_search(query: str, db_path: str, k: int = 5) -> list[str]:
    """Run FTS5-only search via quaid search."""
    result = subprocess.run(
        [QUAID_BIN, "--db", db_path, "search", query, "--json"],
        capture_output=True, text=True, timeout=30
    )
    if result.returncode != 0:
        return []
    try:
        data = json.loads(result.stdout)
        items = data if isinstance(data, list) else data.get("results", [])
        return [r.get("summary", "") for r in items[:k]]
    except (json.JSONDecodeError, KeyError):
        return []


# ── Metrics: token-level F1 ───────────────────────────────────────────────────

def token_f1(prediction: str, ground_truth: str) -> float:
    """Token-level F1 between prediction and ground truth (LoCoMo standard metric)."""
    pred_tokens = set(prediction.lower().split())
    truth_tokens = set(ground_truth.lower().split())
    if not pred_tokens or not truth_tokens:
        return 0.0
    common = pred_tokens & truth_tokens
    precision = len(common) / len(pred_tokens)
    recall = len(common) / len(truth_tokens)
    if precision + recall == 0:
        return 0.0
    return 2 * precision * recall / (precision + recall)


def max_f1(retrieved_texts: list[str], ground_truth: str) -> float:
    """Max token-F1 across retrieved texts."""
    if not retrieved_texts:
        return 0.0
    return max(token_f1(text, ground_truth) for text in retrieved_texts)


# ── Evaluation ────────────────────────────────────────────────────────────────

def extract_qa_pairs(sessions: list[dict]) -> list[dict[str, str]]:
    """Extract QA pairs from LoCoMo sessions."""
    pairs = []
    for session in sessions:
        for qa in session.get("question_answer_pairs", session.get("qa_pairs", [])):
            question = qa.get("question", "")
            answer = qa.get("answer", qa.get("ground_truth", ""))
            if question and answer:
                pairs.append({"question": question, "answer": answer})
    return pairs


def run_evaluation(
    sessions: list[dict],
    db_path: str,
    limit: int | None = None,
    baseline_only: bool = False,
) -> dict[str, Any]:
    qa_pairs = extract_qa_pairs(sessions)
    if limit:
        qa_pairs = qa_pairs[:limit]

    hybrid_f1_scores = []
    fts5_f1_scores = []

    for qa in tqdm(qa_pairs, desc="LoCoMo queries"):
        question = qa["question"]
        answer = qa["answer"]

        # FTS5 baseline
        fts5_results = run_fts5_search(question, db_path)
        fts5_f1_scores.append(max_f1(fts5_results, answer))

        if not baseline_only:
            # Hybrid (FTS5 + vector)
            hybrid_results = run_hybrid_query(question, db_path)
            hybrid_f1_scores.append(max_f1(hybrid_results, answer))

    fts5_mean = float(np.mean(fts5_f1_scores)) if fts5_f1_scores else 0.0
    hybrid_mean = float(np.mean(hybrid_f1_scores)) if hybrid_f1_scores else 0.0

    if baseline_only:
        delta_pct = 0.0
        passed = None
    else:
        delta_pct = (hybrid_mean - fts5_mean) / max(fts5_mean, 1e-9) * 100
        passed = delta_pct >= TARGET_DELTA_F1 * 100

    return {
        "benchmark": "LoCoMo",
        "metric": "token_F1",
        "fts5_baseline_f1": fts5_mean,
        "hybrid_f1": hybrid_mean if not baseline_only else None,
        "delta_pct": delta_pct,
        "target_delta_pct": TARGET_DELTA_F1 * 100,
        "passed": passed,
        "n_questions": len(qa_pairs),
        "baseline_only": baseline_only,
    }


# ── Helpers ───────────────────────────────────────────────────────────────────

def _sanitize_slug(s: str) -> str:
    return "".join(c if c.isalnum() or c in "-_" else "-" for c in s).strip("-")[:64]


# ── CLI ───────────────────────────────────────────────────────────────────────

def main() -> None:
    parser = argparse.ArgumentParser(description="LoCoMo evaluation for Quaid")
    parser.add_argument("--db", default=None, help="Path to memory.db (default: temp file)")
    parser.add_argument("--limit", type=int, default=None, help="Limit number of QA pairs")
    parser.add_argument("--baseline-only", action="store_true", help="Measure FTS5 baseline only")
    parser.add_argument("--no-import", action="store_true", help="Skip import (DB already populated)")
    parser.add_argument("--json", action="store_true", help="Output JSON results")
    args = parser.parse_args()

    if not Path(QUAID_BIN).exists():
        sys.exit(f"quaid binary not found at {QUAID_BIN}. Run: cargo build --release")

    use_temp = args.db is None
    db_path = args.db or tempfile.mktemp(suffix=".db")

    try:
        print("Loading LoCoMo conversations...", file=sys.stderr)
        sessions = load_locomo_data()
        print(f"Loaded {len(sessions)} conversations", file=sys.stderr)

        if not args.no_import:
            subprocess.run([QUAID_BIN, "--db", db_path, "init"], check=True, capture_output=True)
            count = import_conversations(sessions, db_path)
            print(f"Imported {count} conversations", file=sys.stderr)

        print("Running LoCoMo evaluation...", file=sys.stderr)
        results = run_evaluation(
            sessions, db_path, limit=args.limit, baseline_only=args.baseline_only
        )

        if args.json:
            print(json.dumps(results, indent=2))
        else:
            print("\n=== LoCoMo Results ===")
            print(f"Conversations: {results['n_questions']} QA pairs")
            print(f"FTS5 baseline F1: {results['fts5_baseline_f1']:.4f}")
            if not args.baseline_only:
                print(f"Hybrid F1:        {results['hybrid_f1']:.4f}")
                print(f"Delta:            +{results['delta_pct']:.1f}%  (target: +{TARGET_DELTA_F1:.0%})")
                status = "✓ PASS" if results["passed"] else "✗ FAIL"
                print(f"Status:           {status}")

        if results["passed"] is False:
            sys.exit(1)

    finally:
        if use_temp and Path(db_path).exists():
            Path(db_path).unlink()


if __name__ == "__main__":
    main()
