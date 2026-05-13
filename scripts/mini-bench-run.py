#!/usr/bin/env python3
"""
mini-bench-run.py - Run 20 queries against pre-indexed corpus and report score.

FTS queries: test keyword/phrase retrieval edge cases
Semantic queries: test embedding-based recall

Expected result: substring that should appear in at least one top-5 slug or title.
"""

import argparse
import json
import os
import subprocess
import sys
import time

# ─── Query definitions ────────────────────────────────────────────────────────
# Each entry: (query_string, expected_substring_in_slug_or_title, label)

FTS_QUERIES = [
    ("agent memory architecture",        "Agent Memory",        "S01"),
    ("stablecoin regulation",            "Stablecoin Reg",      "S02"),
    ("DeFi liquidity protocol",          "DeFi",                "S03"),
    ("Rust async runtime performance",   "Rust",                "S04"),
    ("vector embedding search",          "vector",              "S05"),
    ("knowledge graph traversal",        "Knowledge Graph",     "S06"),
    ("smart contract security audit",    "Smart Contract",      "S07"),
    ("cross-chain bridge mechanism",     "Cross-chain",         "S08"),
    ("zero knowledge proof circuit",     "Zero Knowledge",      "S09"),
    ("retrieval augmented generation",   "Retrieval",           "S10"),
]

SEMANTIC_QUERIES = [
    ("how do AI agents remember information across multiple sessions",
     "Agent Memory",   "Q01"),
    ("what makes decentralized exchanges different from centralized ones",
     "DeFi",           "Q02"),
    ("techniques for compressing large language model context windows",
     "LLM",            "Q03"),
    ("regulatory challenges for stablecoin issuers in the US",
     "Stablecoin",     "Q04"),
    ("building efficient search over large markdown document collections",
     "vector",         "Q05"),
    ("how to detect contradictions in a knowledge base",
     "Knowledge",      "Q06"),
    ("performance optimisation strategies for Rust web services",
     "Rust",           "Q07"),
    ("blockchain interoperability and cross-chain asset transfers",
     "Cross-chain",    "Q08"),
    ("cryptographic commitments and privacy in smart contracts",
     "Zero Knowledge", "Q09"),
    ("graph-based reasoning and link traversal in document stores",
     "Knowledge Graph", "Q10"),
]


# ─── Runner ───────────────────────────────────────────────────────────────────

def run_query(quaid_bin: str, db_path: str, query: str,
              cmd: str = "search", limit: int = 5) -> list[dict]:
    env = {**os.environ, "QUAID_DB": db_path}
    result = subprocess.run(
        [quaid_bin, cmd, query, "--limit", str(limit), "--json"],
        env=env, capture_output=True, text=True, timeout=15,
    )
    if result.returncode != 0:
        return []
    try:
        data = json.loads(result.stdout)
        return data if isinstance(data, list) else data.get("results", [])
    except Exception:
        return []


def check_hit(results: list[dict], expected: str) -> bool:
    for r in results:
        slug = r.get("slug", "").lower()
        title = r.get("title", "").lower()
        if expected.lower() in slug or expected.lower() in title:
            return True
    return False


def bar(n: int, total: int, width: int = 10) -> str:
    filled = round(n / total * width)
    return "█" * filled + "░" * (width - filled)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--db", default="/tmp/quaid-mini-bench.db")
    parser.add_argument("--quaid", default="./target/release/quaid")
    parser.add_argument("--prev", default="/tmp/quaid-mini-bench-prev.txt")
    args = parser.parse_args()

    quaid = os.path.abspath(args.quaid)
    if not os.path.isfile(quaid):
        print(f"ERROR: quaid binary not found at {quaid}")
        print("Run: cargo build --release")
        sys.exit(1)

    if not os.path.isfile(args.db):
        print(f"ERROR: bench DB not found at {args.db}")
        print("Run: python3 scripts/mini-bench-setup.py")
        sys.exit(1)

    # Load previous score
    prev_score = None
    if os.path.isfile(args.prev):
        try:
            prev_score = int(open(args.prev).read().strip())
        except Exception:
            pass

    fts_results = []
    sem_results = []
    t_start = time.time()

    # FTS
    for query, expected, label in FTS_QUERIES:
        hits = run_query(quaid, args.db, query, cmd="search")
        passed = check_hit(hits, expected)
        fts_results.append((label, passed))

    # Semantic
    for query, expected, label in SEMANTIC_QUERIES:
        hits = run_query(quaid, args.db, query, cmd="query")
        passed = check_hit(hits, expected)
        sem_results.append((label, passed))

    elapsed = time.time() - t_start

    fts_pass = sum(1 for _, p in fts_results if p)
    sem_pass = sum(1 for _, p in sem_results if p)
    total = fts_pass + sem_pass
    max_total = 20

    fts_labels = " ".join(f"{l}{'✓' if p else '✗'}" for l, p in fts_results)
    sem_labels = " ".join(f"{l}{'✓' if p else '✗'}" for l, p in sem_results)

    print()
    print(f"§3 FTS  [{bar(fts_pass, 10)}] {fts_pass}/10   {fts_labels}")
    print(f"§4 Sem  [{bar(sem_pass, 10)}] {sem_pass}/10   {sem_labels}")

    delta_str = ""
    if prev_score is not None:
        delta = total - prev_score
        sign = "+" if delta >= 0 else ""
        delta_str = f"  prev: {prev_score}/{max_total}  delta: {sign}{delta}"

    grade = "✅" if total >= 14 else ("⚠️ " if total >= 10 else "🔴")
    print(f"Total: {total}/{max_total}{delta_str}  ({elapsed:.1f}s)  {grade}")
    print()

    # Save score
    try:
        with open(args.prev, "w") as f:
            f.write(str(total))
    except Exception:
        pass

    # Regression gate: exit 1 if below threshold
    if total < 8:
        print("🔴 REGRESSION: score below gate (8/20)")
        sys.exit(1)


if __name__ == "__main__":
    main()
