#!/usr/bin/env python3
"""
benchmarks/longmemeval_adapter.py

LongMemEval adapter for GigaBrain.

Evaluates multi-session memory retrieval using the LongMemEval benchmark.
Converts LongMemEval sessions to gbrain pages, runs queries through gbrain,
and measures Recall@5 against ground-truth answers.

Target: R@5 >= 85%

Prerequisites:
  - Dataset downloaded: ./benchmarks/prep_datasets.sh longmemeval
  - GigaBrain binary built: cargo build --release
  - Python deps: pip install -r benchmarks/requirements.txt

Usage:
  python benchmarks/longmemeval_adapter.py
  python benchmarks/longmemeval_adapter.py --db /path/to/brain.db
  python benchmarks/longmemeval_adapter.py --limit 100   # evaluate first 100 questions
  python benchmarks/longmemeval_adapter.py --split test  # test | dev

Environment:
  GBRAIN_BIN  — path to gbrain binary (default: ./target/release/gbrain)
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
from tqdm import tqdm

# ── Config ────────────────────────────────────────────────────────────────────

REPO_ROOT = Path(__file__).parent.parent
DATASETS_DIR = Path(os.environ.get("DATASETS_DIR", REPO_ROOT / "benchmarks" / "datasets"))
GBRAIN_BIN = os.environ.get("GBRAIN_BIN", str(REPO_ROOT / "target" / "release" / "gbrain"))
LONGMEMEVAL_DIR = DATASETS_DIR / "longmemeval"

TARGET_RECALL_AT_5 = 0.85

# ── Dataset loader ────────────────────────────────────────────────────────────

def load_longmemeval_sessions(split: str = "test") -> list[dict[str, Any]]:
    """Load LongMemEval sessions from the downloaded dataset."""
    data_dir = LONGMEMEVAL_DIR / "data"
    if not data_dir.exists():
        sys.exit(
            f"LongMemEval data not found at {data_dir}.\n"
            f"Run: ./benchmarks/prep_datasets.sh longmemeval"
        )

    # LongMemEval uses JSON files per split
    candidates = list(data_dir.glob(f"{split}*.json")) + list(data_dir.glob(f"*{split}*.json"))
    if not candidates:
        sys.exit(f"No LongMemEval {split} data found in {data_dir}.")

    sessions = []
    for path in sorted(candidates):
        with open(path) as f:
            data = json.load(f)
        if isinstance(data, list):
            sessions.extend(data)
        elif isinstance(data, dict) and "sessions" in data:
            sessions.extend(data["sessions"])
    return sessions


# ── Importer: sessions → gbrain pages ────────────────────────────────────────

def sessions_to_pages(sessions: list[dict], db_path: str, gbrain_bin: str) -> int:
    """Import LongMemEval sessions as gbrain pages. Returns count of imported pages."""
    with tempfile.TemporaryDirectory() as tmpdir:
        pages_dir = Path(tmpdir) / "sessions"
        pages_dir.mkdir()

        for session in sessions:
            session_id = session.get("session_id", session.get("id", "unknown"))
            slug = f"sessions/{_sanitize_slug(str(session_id))}"

            # Convert session turns to gbrain timeline format
            turns = session.get("conversation", session.get("turns", []))
            timeline_lines = []
            for turn in turns:
                role = turn.get("role", "user")
                content = turn.get("content", "")
                date = turn.get("timestamp", "2024-01")
                timeline_lines.append(f"- **{date}** | {role} — {content[:200]}")

            content = "\n".join([
                "---",
                f"slug: {slug}",
                f"title: Session {session_id}",
                "type: session",
                "wing: sessions",
                "---",
                f"# Session {session_id}",
                "",
                session.get("summary", f"Conversation session {session_id}."),
                "---",
                *timeline_lines,
            ])

            page_path = pages_dir / f"{_sanitize_slug(str(session_id))}.md"
            page_path.write_text(content)

        # Import via gbrain CLI
        result = subprocess.run(
            [gbrain_bin, "--db", db_path, "import", str(pages_dir)],
            capture_output=True, text=True
        )
        if result.returncode != 0:
            print(f"Import warning: {result.stderr}", file=sys.stderr)

    # Count pages
    count_result = subprocess.run(
        [gbrain_bin, "--db", db_path, "stats", "--json"],
        capture_output=True, text=True
    )
    if count_result.returncode == 0:
        try:
            stats = json.loads(count_result.stdout)
            return stats.get("page_count", 0)
        except json.JSONDecodeError:
            pass
    return len(sessions)


# ── Retrieval ─────────────────────────────────────────────────────────────────

def run_query(query: str, db_path: str, gbrain_bin: str, k: int = 5) -> list[str]:
    """Run a query through gbrain and return top-k slugs."""
    result = subprocess.run(
        [gbrain_bin, "--db", db_path, "query", query, "--json"],
        capture_output=True, text=True, timeout=30
    )
    if result.returncode != 0:
        return []
    try:
        data = json.loads(result.stdout)
        results = data if isinstance(data, list) else data.get("results", [])
        return [r["slug"] for r in results[:k]]
    except (json.JSONDecodeError, KeyError):
        return []


# ── Evaluation ────────────────────────────────────────────────────────────────

def compute_recall_at_k(retrieved: list[str], relevant: list[str], k: int = 5) -> float:
    """Recall@k: fraction of relevant items found in top-k retrieved."""
    if not relevant:
        return 1.0
    retrieved_k = set(retrieved[:k])
    found = sum(1 for r in relevant if any(r in slug for slug in retrieved_k))
    return found / len(relevant)


def evaluate(
    sessions: list[dict[str, Any]],
    db_path: str,
    gbrain_bin: str,
    limit: int | None = None,
) -> dict[str, Any]:
    """Run full LongMemEval evaluation. Returns metric summary."""
    questions = []
    for session in sessions:
        for qa in session.get("questions", session.get("qa_pairs", [])):
            questions.append({
                "session_id": session.get("session_id", session.get("id")),
                "question": qa.get("question", qa.get("query", "")),
                "answer": qa.get("answer", ""),
                "evidence_slugs": qa.get("evidence_slugs", []),
            })

    if limit:
        questions = questions[:limit]

    recalls = []
    for qa in tqdm(questions, desc="LongMemEval queries"):
        query = qa["question"]
        expected_slugs = qa["evidence_slugs"]
        retrieved = run_query(query, db_path, gbrain_bin, k=5)
        r5 = compute_recall_at_k(retrieved, expected_slugs, k=5)
        recalls.append(r5)

    mean_r5 = float(np.mean(recalls)) if recalls else 0.0
    return {
        "benchmark": "LongMemEval",
        "metric": "R@5",
        "value": mean_r5,
        "target": TARGET_RECALL_AT_5,
        "passed": mean_r5 >= TARGET_RECALL_AT_5,
        "n_questions": len(questions),
        "n_sessions": len(sessions),
        "per_question_recalls": recalls,
    }


# ── Helpers ───────────────────────────────────────────────────────────────────

def _sanitize_slug(s: str) -> str:
    return "".join(c if c.isalnum() or c in "-_" else "-" for c in s).strip("-")[:64]


# ── CLI ───────────────────────────────────────────────────────────────────────

def main() -> None:
    parser = argparse.ArgumentParser(description="LongMemEval adapter for GigaBrain")
    parser.add_argument("--db", default=":memory:", help="Path to brain.db (default: temp)")
    parser.add_argument("--split", default="test", choices=["test", "dev", "train"])
    parser.add_argument("--limit", type=int, default=None, help="Limit number of questions")
    parser.add_argument("--no-import", action="store_true", help="Skip import (DB already populated)")
    parser.add_argument("--json", action="store_true", help="Output JSON results")
    args = parser.parse_args()

    if not Path(GBRAIN_BIN).exists():
        sys.exit(f"gbrain binary not found at {GBRAIN_BIN}. Run: cargo build --release")

    use_temp_db = args.db == ":memory:"
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) if use_temp_db else open(os.devnull) as _f:
        db_path = tempfile.mktemp(suffix=".db") if use_temp_db else args.db

    try:
        print("Loading LongMemEval sessions...", file=sys.stderr)
        sessions = load_longmemeval_sessions(args.split)
        print(f"Loaded {len(sessions)} sessions", file=sys.stderr)

        if not args.no_import:
            print("Importing sessions into gbrain...", file=sys.stderr)
            # Initialize DB first
            subprocess.run([GBRAIN_BIN, "--db", db_path, "init"], check=True, capture_output=True)
            page_count = sessions_to_pages(sessions, db_path, GBRAIN_BIN)
            print(f"Imported {page_count} pages", file=sys.stderr)

        print("Running evaluation...", file=sys.stderr)
        results = evaluate(sessions, db_path, GBRAIN_BIN, limit=args.limit)

        if args.json:
            print(json.dumps(results, indent=2))
        else:
            print(f"\n=== LongMemEval Results ===")
            print(f"Sessions:   {results['n_sessions']}")
            print(f"Questions:  {results['n_questions']}")
            print(f"R@5:        {results['value']:.4f}  (target: ≥{TARGET_RECALL_AT_5:.0%})")
            status = "✓ PASS" if results["passed"] else "✗ FAIL"
            print(f"Status:     {status}")

        sys.exit(0 if results["passed"] else 1)

    finally:
        if use_temp_db and Path(db_path).exists():
            Path(db_path).unlink()


if __name__ == "__main__":
    main()
