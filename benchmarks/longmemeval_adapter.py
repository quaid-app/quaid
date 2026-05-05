#!/usr/bin/env python3
"""
benchmarks/longmemeval_adapter.py

LongMemEval adapter for Quaid.

Evaluates multi-session memory retrieval using the LongMemEval benchmark.
Converts LongMemEval sessions to quaid pages, runs queries through quaid,
and measures Recall@5 against ground-truth answers.

Target: R@5 >= 85%

Prerequisites:
  - Dataset downloaded: ./benchmarks/prep_datasets.sh longmemeval
  - Quaid binary built: cargo build --release
  - Python deps: pip install -r benchmarks/requirements.txt

Usage:
  python benchmarks/longmemeval_adapter.py
  python benchmarks/longmemeval_adapter.py --db /path/to/memory.db
  python benchmarks/longmemeval_adapter.py --limit 100   # evaluate first 100 questions
  python benchmarks/longmemeval_adapter.py --split test  # test | dev

Environment:
  QUAID_BIN  — path to quaid binary (default: ./target/release/quaid)
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

from conversation_memory_common import (
    ConversationBenchmarkError,
    answer_hit_at_k,
    configure_workspace,
    coerce_role,
    fact_page_count,
    ingest_turns,
    max_f1,
    normalize_answer,
    ranked_fact_texts,
    serve_runtime,
    wait_for_extraction_completion,
)

# ── Config ────────────────────────────────────────────────────────────────────

REPO_ROOT = Path(__file__).parent.parent
DATASETS_DIR = Path(os.environ.get("DATASETS_DIR", REPO_ROOT / "benchmarks" / "datasets"))
QUAID_BIN = os.environ.get("QUAID_BIN", str(REPO_ROOT / "target" / "release" / "quaid"))
LONGMEMEVAL_DIR = DATASETS_DIR / "longmemeval"

TARGET_RECALL_AT_5 = 0.85
TARGET_CONVERSATION_MEMORY_HIT_AT_5 = 0.40

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


# ── Importer: sessions → quaid pages ────────────────────────────────────────

def sessions_to_pages(sessions: list[dict], db_path: str, quaid_bin: str) -> int:
    """Import LongMemEval sessions as quaid pages. Returns count of imported pages."""
    with tempfile.TemporaryDirectory() as tmpdir:
        pages_dir = Path(tmpdir) / "sessions"
        pages_dir.mkdir()

        for session in sessions:
            session_id = session.get("session_id", session.get("id", "unknown"))
            slug = f"sessions/{_sanitize_slug(str(session_id))}"

            # Convert session turns to quaid timeline format
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

        # Import via quaid CLI
        result = subprocess.run(
            [quaid_bin, "--db", db_path, "import", str(pages_dir)],
            capture_output=True, text=True
        )
        if result.returncode != 0:
            print(f"Import warning: {result.stderr}", file=sys.stderr)

    # Count pages
    count_result = subprocess.run(
        [quaid_bin, "--db", db_path, "stats", "--json"],
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

def run_query(query: str, db_path: str, quaid_bin: str, k: int = 5) -> list[str]:
    """Run a query through quaid and return top-k slugs."""
    result = subprocess.run(
        [quaid_bin, "--db", db_path, "query", query, "--json"],
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
    quaid_bin: str,
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
        retrieved = run_query(query, db_path, quaid_bin, k=5)
        r5 = compute_recall_at_k(retrieved, expected_slugs, k=5)
        recalls.append(r5)

    mean_r5 = float(np.mean(recalls)) if recalls else 0.0
    return {
        "benchmark": "LongMemEval",
        "mode": "page_import",
        "metric": "R@5",
        "value": mean_r5,
        "target": TARGET_RECALL_AT_5,
        "passed": mean_r5 >= TARGET_RECALL_AT_5,
        "n_questions": len(questions),
        "n_sessions": len(sessions),
        "per_question_recalls": recalls,
    }


def normalize_turns(session: dict[str, Any]) -> list[dict[str, str]]:
    """Normalize LongMemEval turns onto Quaid's conversation-memory surface."""
    normalized: list[dict[str, str]] = []
    speaker_map: dict[str, str] = {}
    for turn in session.get("conversation", session.get("turns", [])):
        role = turn.get("role", turn.get("speaker", "participant"))
        content = str(turn.get("content", turn.get("text", ""))).strip()
        if not content:
            continue
        normalized.append(
            {
                "role": coerce_role(str(role), speaker_map),
                "content": content,
                "timestamp": str(turn.get("timestamp", "")).strip(),
            }
        )
    return normalized


def run_conversation_memory_evaluation(
    sessions: list[dict[str, Any]],
    *,
    db_path: str,
    workspace_dir: Path,
    limit: int | None = None,
    model_alias: str | None = None,
    raw_limit: int = 20,
    wait_timeout: int = 900,
    answer_f1_threshold: float = 0.5,
) -> dict[str, Any]:
    """
    Exercise the real extraction path and score answer-hit@5 over fact pages.

    LongMemEval's session-level evidence slugs do not align with extracted fact-page
    slugs, so the truthful in-repo proxy here is answer-hit@5 using token-F1 ≥ 0.5.
    """
    configure_workspace(db_path, workspace_dir, model_alias=model_alias)
    with serve_runtime(db_path):
        for session_index, session in enumerate(sessions):
            session_id = session.get("session_id", session.get("id", f"longmemeval-{session_index}"))
            ingest_turns(
                db_path,
                str(session_id),
                normalize_turns(session),
                session_index=session_index,
            )
        queue_counts = wait_for_extraction_completion(db_path, timeout_s=wait_timeout)

    questions = []
    for session in sessions:
        for qa in session.get("questions", session.get("qa_pairs", [])):
            answer = normalize_answer(qa.get("answer", ""))
            question = qa.get("question", qa.get("query", ""))
            if question and answer:
                questions.append(
                    {
                        "question": question,
                        "answer": answer,
                    }
                )

    if limit:
        questions = questions[:limit]

    hits = []
    f1_scores = []
    for qa in tqdm(questions, desc="LongMemEval §8 queries"):
        retrieved = ranked_fact_texts(
            db_path,
            qa["question"],
            mode="hybrid",
            k=5,
            limit=raw_limit,
        )
        f1_scores.append(max_f1(retrieved, qa["answer"]))
        hits.append(
            answer_hit_at_k(
                retrieved,
                qa["answer"],
                f1_threshold=answer_f1_threshold,
            )
        )

    mean_hit = float(np.mean(hits)) if hits else 0.0
    mean_f1 = float(np.mean(f1_scores)) if f1_scores else 0.0
    return {
        "benchmark": "LongMemEval",
        "mode": "conversation_memory",
        "metric": "answer_hit_at_5",
        "metric_note": "Proxy for extracted fact pages; counts a hit when top-5 max token-F1 ≥ 0.5 against the gold answer.",
        "value": mean_hit,
        "score_pct": mean_hit * 100,
        "mean_max_f1": mean_f1,
        "target": TARGET_CONVERSATION_MEMORY_HIT_AT_5,
        "passed_target": mean_hit >= TARGET_CONVERSATION_MEMORY_HIT_AT_5,
        "n_questions": len(questions),
        "n_sessions": len(sessions),
        "fact_page_count": fact_page_count(db_path),
        "queue_counts": queue_counts,
        "per_question_hits": hits,
    }


# ── Helpers ───────────────────────────────────────────────────────────────────

def _sanitize_slug(s: str) -> str:
    return "".join(c if c.isalnum() or c in "-_" else "-" for c in s).strip("-")[:64]


# ── CLI ───────────────────────────────────────────────────────────────────────

def main() -> None:
    parser = argparse.ArgumentParser(description="LongMemEval adapter for Quaid")
    parser.add_argument(
        "--mode",
        choices=["page-import", "conversation-memory"],
        default="page-import",
        help="Evaluation surface: legacy page import or DAB §8 conversation-memory path",
    )
    parser.add_argument("--db", default=":memory:", help="Path to memory.db (default: temp)")
    parser.add_argument("--split", default="test", choices=["test", "dev", "train"])
    parser.add_argument("--limit", type=int, default=None, help="Limit number of questions")
    parser.add_argument("--no-import", action="store_true", help="Skip import (DB already populated)")
    parser.add_argument("--json", action="store_true", help="Output JSON results")
    parser.add_argument("--work-dir", default=None, help="Scratch workspace for conversation-memory mode")
    parser.add_argument("--model-alias", default=None, help="Override extraction.model_alias for benchmark setup")
    parser.add_argument("--wait-timeout", type=int, default=900, help="Seconds to wait for extraction to finish")
    parser.add_argument("--raw-limit", type=int, default=20, help="Initial retrieval depth before filtering to fact pages")
    args = parser.parse_args()

    if not Path(QUAID_BIN).exists():
        sys.exit(f"quaid binary not found at {QUAID_BIN}. Run: cargo build --release")

    use_temp_db = args.db == ":memory:"
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) if use_temp_db else open(os.devnull) as _f:
        db_path = tempfile.mktemp(suffix=".db") if use_temp_db else args.db

    try:
        print("Loading LongMemEval sessions...", file=sys.stderr)
        sessions = load_longmemeval_sessions(args.split)
        print(f"Loaded {len(sessions)} sessions", file=sys.stderr)

        if args.mode == "conversation-memory":
            workspace_root = (
                Path(args.work_dir)
                if args.work_dir
                else Path(tempfile.mkdtemp(prefix="quaid-longmemeval-"))
            )
            print("Running LongMemEval DAB §8 conversation-memory evaluation...", file=sys.stderr)
            results = run_conversation_memory_evaluation(
                sessions,
                db_path=db_path,
                workspace_dir=workspace_root,
                limit=args.limit,
                model_alias=args.model_alias,
                raw_limit=args.raw_limit,
                wait_timeout=args.wait_timeout,
            )
        else:
            if not args.no_import:
                print("Importing sessions into quaid...", file=sys.stderr)
                # Initialize DB first
                subprocess.run([QUAID_BIN, "--db", db_path, "init"], check=True, capture_output=True)
                page_count = sessions_to_pages(sessions, db_path, QUAID_BIN)
                print(f"Imported {page_count} pages", file=sys.stderr)

            print("Running evaluation...", file=sys.stderr)
            results = evaluate(sessions, db_path, QUAID_BIN, limit=args.limit)

        if args.json:
            print(json.dumps(results, indent=2))
        else:
            print(f"\n=== LongMemEval Results ===")
            print(f"Sessions:   {results['n_sessions']}")
            print(f"Questions:  {results['n_questions']}")
            if results["mode"] == "conversation_memory":
                print(
                    f"Answer hit@5: {results['value']:.4f}  "
                    f"(target: ≥{TARGET_CONVERSATION_MEMORY_HIT_AT_5:.0%})"
                )
                print(f"Mean max F1:  {results['mean_max_f1']:.4f}")
                print(f"Fact pages:   {results['fact_page_count']}")
                print(f"Queue counts: {results['queue_counts']}")
                status = "✓ PASS" if results["passed_target"] else "✗ FAIL"
                print(f"Target:       {status}")
            else:
                print(f"R@5:        {results['value']:.4f}  (target: ≥{TARGET_RECALL_AT_5:.0%})")
                status = "✓ PASS" if results["passed"] else "✗ FAIL"
                print(f"Status:     {status}")

        if results.get("passed") is False or results.get("passed_target") is False:
            sys.exit(1)
        sys.exit(0)

    except ConversationBenchmarkError as exc:
        sys.exit(str(exc))
    finally:
        if use_temp_db and Path(db_path).exists():
            Path(db_path).unlink()


if __name__ == "__main__":
    main()
