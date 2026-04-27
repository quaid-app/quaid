#!/usr/bin/env python3
"""
benchmarks/ragas_eval.py

Ragas evaluation for Quaid progressive retrieval.

Measures context_precision, context_recall, and faithfulness for
`quaid query --depth auto` using Ragas as the LLM-based judge.

This is an ADVISORY benchmark — results inform quality improvements but
do not block release gates.

Prerequisites:
  - Quaid binary built: cargo build --release
  - Python deps: pip install -r benchmarks/requirements.txt
  - LLM judge: OpenAI API key OR local Ollama instance

Usage:
  # With OpenAI:
  OPENAI_API_KEY=sk-... python benchmarks/ragas_eval.py

  # With Ollama (llama3.2 or similar):
  OLLAMA_BASE_URL=http://localhost:11434 python benchmarks/ragas_eval.py --llm ollama

  # Point at an existing populated brain:
  python benchmarks/ragas_eval.py --db ~/memory.db

  # Limit queries:
  python benchmarks/ragas_eval.py --limit 20

Environment:
  OPENAI_API_KEY    — OpenAI API key (required unless --llm ollama)
  OLLAMA_BASE_URL   — Ollama API base URL (default: http://localhost:11434)
  OLLAMA_MODEL      — Ollama model name (default: llama3.2)
  QUAID_BIN        — path to quaid binary (default: ./target/release/quaid)
"""

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

# ── Config ────────────────────────────────────────────────────────────────────

REPO_ROOT = Path(__file__).parent.parent
QUAID_BIN = os.environ.get("QUAID_BIN", str(REPO_ROOT / "target" / "release" / "quaid"))

OLLAMA_BASE_URL = os.environ.get("OLLAMA_BASE_URL", "http://localhost:11434")
OLLAMA_MODEL = os.environ.get("OLLAMA_MODEL", "llama3.2")

# Synthetic evaluation query set (used when no brain is provided)
DEFAULT_EVAL_QUERIES = [
    "who founded brex",
    "what is quaid used for",
    "knowledge management with sqlite",
    "corporate card fintech startup history",
    "personal knowledge base embeddings",
    "rust sqlite vector search architecture",
    "developer productivity tools apis",
    "entrepreneur yc alumni brex cto",
    "temporal knowledge management compiled truth",
    "hybrid semantic search retrieval",
]


# ── LLM provider setup ────────────────────────────────────────────────────────

def build_llm(provider: str) -> Any:
    """Build LangChain LLM object for Ragas judge."""
    if provider == "openai":
        from langchain_openai import ChatOpenAI
        api_key = os.environ.get("OPENAI_API_KEY")
        if not api_key:
            sys.exit("OPENAI_API_KEY not set. Use --llm ollama for local evaluation.")
        return ChatOpenAI(model="gpt-4o-mini", temperature=0, api_key=api_key)
    elif provider == "ollama":
        from langchain_community.chat_models import ChatOllama
        return ChatOllama(
            base_url=OLLAMA_BASE_URL,
            model=OLLAMA_MODEL,
            temperature=0,
        )
    else:
        sys.exit(f"Unknown LLM provider: {provider}. Use 'openai' or 'ollama'.")


def build_embeddings(provider: str) -> Any:
    """Build LangChain embeddings for Ragas context precision."""
    if provider == "openai":
        from langchain_openai import OpenAIEmbeddings
        return OpenAIEmbeddings(model="text-embedding-3-small")
    elif provider == "ollama":
        from langchain_community.embeddings import OllamaEmbeddings
        return OllamaEmbeddings(base_url=OLLAMA_BASE_URL, model=OLLAMA_MODEL)
    else:
        sys.exit(f"Unknown embeddings provider: {provider}")


# ── Retrieval ─────────────────────────────────────────────────────────────────

def run_progressive_query(query: str, db_path: str, depth: str = "auto") -> dict[str, Any]:
    """Run progressive retrieval query and return result + context."""
    result = subprocess.run(
        [QUAID_BIN, "--db", db_path, "query", query, "--json"],
        capture_output=True, text=True, timeout=60
    )
    if result.returncode != 0:
        return {"query": query, "answer": "", "contexts": []}

    try:
        data = json.loads(result.stdout)
        items = data if isinstance(data, list) else data.get("results", [])

        contexts = []
        for item in items[:5]:
            ctx = item.get("compiled_truth", "") or item.get("summary", "")
            if ctx:
                contexts.append(ctx)

        # Synthesize a simple answer from top result
        answer = contexts[0][:500] if contexts else ""
        return {"query": query, "answer": answer, "contexts": contexts}

    except (json.JSONDecodeError, KeyError):
        return {"query": query, "answer": "", "contexts": []}


# ── Ragas evaluation ──────────────────────────────────────────────────────────

def run_ragas_evaluation(
    retrieval_results: list[dict[str, Any]],
    llm: Any,
    embeddings: Any,
) -> dict[str, Any]:
    """Evaluate retrieval results with Ragas metrics."""
    from datasets import Dataset
    from ragas import evaluate as ragas_evaluate
    from ragas.metrics import (
        answer_relevancy,
        context_precision,
        context_recall,
        faithfulness,
    )

    # Build Ragas dataset
    eval_data = {
        "question": [],
        "answer": [],
        "contexts": [],
        "ground_truth": [],
    }

    for r in retrieval_results:
        if r["contexts"]:  # Skip queries with no context
            eval_data["question"].append(r["query"])
            eval_data["answer"].append(r["answer"])
            eval_data["contexts"].append(r["contexts"])
            # Use query as ground truth (advisory — no labeled GT available)
            eval_data["ground_truth"].append(r["query"])

    if not eval_data["question"]:
        return {
            "error": "No valid retrieval results to evaluate",
            "n_evaluated": 0,
        }

    dataset = Dataset.from_dict(eval_data)

    scores = ragas_evaluate(
        dataset,
        metrics=[context_precision, context_recall, faithfulness, answer_relevancy],
        llm=llm,
        embeddings=embeddings,
        raise_exceptions=False,
    )

    return {
        "benchmark": "Ragas",
        "n_evaluated": len(eval_data["question"]),
        "metrics": {
            "context_precision": float(scores["context_precision"]),
            "context_recall": float(scores["context_recall"]),
            "faithfulness": float(scores["faithfulness"]),
            "answer_relevancy": float(scores["answer_relevancy"]),
        },
        "advisory": True,
        "note": "Advisory metric — no release gate threshold. Use to track quality over time.",
    }


# ── CLI ───────────────────────────────────────────────────────────────────────

def main() -> None:
    parser = argparse.ArgumentParser(description="Ragas evaluation for Quaid")
    parser.add_argument("--db", default=None, help="Path to memory.db (uses fixtures if not set)")
    parser.add_argument("--llm", default="openai", choices=["openai", "ollama"], help="LLM judge")
    parser.add_argument("--limit", type=int, default=None, help="Limit number of queries")
    parser.add_argument("--queries-file", default=None, help="JSON file with query list")
    parser.add_argument("--json", action="store_true", help="JSON output")
    parser.add_argument("--dry-run", action="store_true",
                        help="Show queries without running Ragas (no API calls)")
    args = parser.parse_args()

    if not Path(QUAID_BIN).exists():
        sys.exit(f"quaid binary not found at {QUAID_BIN}. Run: cargo build --release")

    # Load queries
    if args.queries_file:
        with open(args.queries_file) as f:
            queries = json.load(f)
    else:
        queries = DEFAULT_EVAL_QUERIES

    if args.limit:
        queries = queries[:args.limit]

    use_temp_db = args.db is None
    db_path = args.db or tempfile.mktemp(suffix=".db")

    try:
        if use_temp_db:
            # Populate with fixture pages for demo evaluation
            subprocess.run([QUAID_BIN, "--db", db_path, "init"], check=True, capture_output=True)
            fixtures_dir = REPO_ROOT / "tests" / "fixtures"
            if fixtures_dir.exists():
                subprocess.run(
                    [QUAID_BIN, "--db", db_path, "import", str(fixtures_dir)],
                    capture_output=True,
                )

        print(f"Running {len(queries)} progressive queries...", file=sys.stderr)
        retrieval_results = []
        for query in queries:
            result = run_progressive_query(query, db_path)
            retrieval_results.append(result)
            print(f"  [{len(retrieval_results)}/{len(queries)}] '{query}' → {len(result['contexts'])} contexts",
                  file=sys.stderr)

        if args.dry_run:
            print("Dry run complete — skipping Ragas LLM evaluation.")
            if args.json:
                print(json.dumps({"dry_run": True, "queries": len(queries), "results": retrieval_results}, indent=2))
            return

        print(f"Running Ragas evaluation (LLM: {args.llm})...", file=sys.stderr)
        llm = build_llm(args.llm)
        embeddings = build_embeddings(args.llm)
        ragas_results = run_ragas_evaluation(retrieval_results, llm, embeddings)

        if args.json:
            print(json.dumps(ragas_results, indent=2))
        else:
            print("\n=== Ragas Advisory Results ===")
            if "error" in ragas_results:
                print(f"Error: {ragas_results['error']}")
            else:
                m = ragas_results["metrics"]
                print(f"Evaluated:         {ragas_results['n_evaluated']} queries")
                print(f"context_precision: {m['context_precision']:.4f}")
                print(f"context_recall:    {m['context_recall']:.4f}")
                print(f"faithfulness:      {m['faithfulness']:.4f}")
                print(f"answer_relevancy:  {m['answer_relevancy']:.4f}")
                print(f"\n[Advisory] No gate threshold — use to track quality over time.")

    finally:
        if use_temp_db and Path(db_path).exists():
            Path(db_path).unlink()


if __name__ == "__main__":
    main()
