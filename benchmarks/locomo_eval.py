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
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

import numpy as np
from tqdm import tqdm

from conversation_memory_common import (
    ConversationBenchmarkError,
    configure_workspace,
    coerce_role,
    fact_page_count,
    ingest_turns,
    max_f1,
    ranked_fact_texts,
    serve_runtime,
    wait_for_extraction_completion,
)

# ── Config ────────────────────────────────────────────────────────────────────

REPO_ROOT = Path(__file__).parent.parent
DATASETS_DIR = Path(os.environ.get("DATASETS_DIR", REPO_ROOT / "benchmarks" / "datasets"))
QUAID_BIN = os.environ.get("QUAID_BIN", str(REPO_ROOT / "target" / "release" / "quaid"))
LOCOMO_DIR = DATASETS_DIR / "locomo"

TARGET_DELTA_F1 = 0.30  # +30% F1 over FTS5 baseline
TARGET_CONVERSATION_MEMORY_F1 = 0.40

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
        "mode": "page_import",
        "metric": "token_F1",
        "fts5_baseline_f1": fts5_mean,
        "hybrid_f1": hybrid_mean if not baseline_only else None,
        "delta_pct": delta_pct,
        "target_delta_pct": TARGET_DELTA_F1 * 100,
        "passed": passed,
        "n_questions": len(qa_pairs),
        "baseline_only": baseline_only,
    }


def normalize_turns(session: dict[str, Any]) -> list[dict[str, str]]:
    """Normalize LoCoMo turns onto Quaid's user/assistant conversation surface."""
    normalized: list[dict[str, str]] = []
    speaker_map: dict[str, str] = {}
    for turn in session.get("conversation", session.get("utterances", [])):
        speaker = turn.get("speaker", turn.get("role", "participant"))
        content = str(turn.get("text", turn.get("content", ""))).strip()
        if not content:
            continue
        normalized.append(
            {
                "role": coerce_role(str(speaker), speaker_map),
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
) -> dict[str, Any]:
    """Exercise the full conversation-memory path and score fact-page retrieval."""
    configure_workspace(db_path, workspace_dir, model_alias=model_alias)
    with serve_runtime(db_path):
        for session_index, session in enumerate(sessions):
            session_id = session.get("conversation_id", session.get("id", f"locomo-{session_index}"))
            ingest_turns(
                db_path,
                str(session_id),
                normalize_turns(session),
                session_index=session_index,
            )
        queue_counts = wait_for_extraction_completion(db_path, timeout_s=wait_timeout)

    qa_pairs = extract_qa_pairs(sessions)
    if limit:
        qa_pairs = qa_pairs[:limit]

    hybrid_f1_scores = []
    fts5_f1_scores = []

    for qa in tqdm(qa_pairs, desc="LoCoMo §8 queries"):
        question = qa["question"]
        answer = qa["answer"]
        fts5_results = ranked_fact_texts(db_path, question, mode="fts5", k=5, limit=raw_limit)
        hybrid_results = ranked_fact_texts(db_path, question, mode="hybrid", k=5, limit=raw_limit)
        fts5_f1_scores.append(max_f1(fts5_results, answer))
        hybrid_f1_scores.append(max_f1(hybrid_results, answer))

    fts5_mean = float(np.mean(fts5_f1_scores)) if fts5_f1_scores else 0.0
    hybrid_mean = float(np.mean(hybrid_f1_scores)) if hybrid_f1_scores else 0.0
    delta_pct = (hybrid_mean - fts5_mean) / max(fts5_mean, 1e-9) * 100

    return {
        "benchmark": "LoCoMo",
        "mode": "conversation_memory",
        "metric": "token_F1",
        "fts5_baseline_f1": fts5_mean,
        "hybrid_f1": hybrid_mean,
        "score_pct": hybrid_mean * 100,
        "delta_pct": delta_pct,
        "target_score_pct": TARGET_CONVERSATION_MEMORY_F1 * 100,
        "passed_target": hybrid_mean >= TARGET_CONVERSATION_MEMORY_F1,
        "n_questions": len(qa_pairs),
        "fact_page_count": fact_page_count(db_path),
        "queue_counts": queue_counts,
        "baseline_only": False,
    }


# ── Helpers ───────────────────────────────────────────────────────────────────

def _sanitize_slug(s: str) -> str:
    return "".join(c if c.isalnum() or c in "-_" else "-" for c in s).strip("-")[:64]


# ── CLI ───────────────────────────────────────────────────────────────────────

def main() -> None:
    parser = argparse.ArgumentParser(description="LoCoMo evaluation for Quaid")
    parser.add_argument(
        "--mode",
        choices=["page-import", "conversation-memory"],
        default="page-import",
        help="Evaluation surface: legacy page import or DAB §8 conversation-memory path",
    )
    parser.add_argument("--db", default=None, help="Path to memory.db (default: temp file)")
    parser.add_argument("--limit", type=int, default=None, help="Limit number of QA pairs")
    parser.add_argument("--baseline-only", action="store_true", help="Measure FTS5 baseline only")
    parser.add_argument("--no-import", action="store_true", help="Skip import (DB already populated)")
    parser.add_argument("--json", action="store_true", help="Output JSON results")
    parser.add_argument("--work-dir", default=None, help="Scratch workspace for conversation-memory mode")
    parser.add_argument("--model-alias", default=None, help="Override extraction.model_alias for benchmark setup")
    parser.add_argument("--wait-timeout", type=int, default=900, help="Seconds to wait for extraction to finish")
    parser.add_argument("--raw-limit", type=int, default=20, help="Initial retrieval depth before filtering to fact pages")
    args = parser.parse_args()

    if not Path(QUAID_BIN).exists():
        sys.exit(f"quaid binary not found at {QUAID_BIN}. Run: cargo build --release")

    if args.mode == "conversation-memory" and args.baseline_only:
        sys.exit("--baseline-only is only supported in page-import mode.")

    use_temp_db = args.db is None
    db_path = args.db or tempfile.mktemp(suffix=".db")

    temp_workspace = None

    try:
        print("Loading LoCoMo conversations...", file=sys.stderr)
        sessions = load_locomo_data()
        print(f"Loaded {len(sessions)} conversations", file=sys.stderr)

        if args.mode == "conversation-memory":
            if args.work_dir:
                workspace_root = Path(args.work_dir)
            else:
                temp_workspace = tempfile.mkdtemp(prefix="quaid-locomo-")
                workspace_root = Path(temp_workspace)
            print(
                "Running LoCoMo DAB §8 conversation-memory evaluation...",
                file=sys.stderr,
            )
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
            if results["mode"] == "conversation_memory":
                print(f"Hybrid fact F1:   {results['hybrid_f1']:.4f}")
                print(f"Fact pages:       {results['fact_page_count']}")
                print(f"Queue counts:     {results['queue_counts']}")
                print(f"Target:           ≥{TARGET_CONVERSATION_MEMORY_F1:.0%}")
                status = "✓ PASS" if results["passed_target"] else "✗ FAIL"
                print(f"Target status:    {status}")
            elif not args.baseline_only:
                print(f"Hybrid F1:        {results['hybrid_f1']:.4f}")
                print(f"Delta:            +{results['delta_pct']:.1f}%  (target: +{TARGET_DELTA_F1:.0%})")
                status = "✓ PASS" if results["passed"] else "✗ FAIL"
                print(f"Status:           {status}")

        if results.get("passed") is False or results.get("passed_target") is False:
            sys.exit(1)

    except ConversationBenchmarkError as exc:
        sys.exit(str(exc))
    finally:
        if use_temp_db and Path(db_path).exists():
            Path(db_path).unlink()
        if temp_workspace:
            shutil.rmtree(temp_workspace, ignore_errors=True)


if __name__ == "__main__":
    main()
