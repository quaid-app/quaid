#!/usr/bin/env python3
"""
mini-bench-setup.py - One-time corpus indexing for mini-bench.
Run once; the DB is reused on every bench run.
"""

import os
import subprocess
import sys
import time

BENCH_DB = "/tmp/quaid-mini-bench.db"
CORPUS_DIR = "/tmp/quaid-bench-corpus"
QUAID = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                     "target", "release", "quaid")

env = {**os.environ, "QUAID_DB": BENCH_DB}


def run(args, **kwargs):
    return subprocess.run([QUAID] + args, env=env, capture_output=True, text=True, **kwargs)


def main():
    if not os.path.isdir(CORPUS_DIR):
        print(f"ERROR: corpus not found at {CORPUS_DIR}")
        print("Run the DAB corpus generator first:")
        print("  cd ~/repos/quaid-evals && python3 benchmarks/dab/generate_corpus.py")
        sys.exit(1)

    if os.path.exists(BENCH_DB):
        os.unlink(BENCH_DB)

    print(f"Initialising DB at {BENCH_DB}...")
    r = run(["init", BENCH_DB])
    if r.returncode != 0:
        print(f"init failed: {r.stderr}")
        sys.exit(1)

    print(f"Adding corpus collection from {CORPUS_DIR}...")
    r = run(["collection", "add", "docs", CORPUS_DIR])
    if r.returncode != 0:
        print(f"collection add failed: {r.stderr}")
        sys.exit(1)

    print("Syncing collection...")
    r = run(["collection", "sync", "docs"])
    if r.returncode != 0:
        print(f"sync failed: {r.stderr}")
        sys.exit(1)

    print("Generating embeddings (this takes ~60-120s on first run)...")
    t0 = time.time()
    r = run(["embed", "--all"], timeout=300)
    elapsed = time.time() - t0
    if r.returncode != 0:
        print(f"embed failed: {r.stderr}")
        sys.exit(1)

    print(f"Done in {elapsed:.0f}s. Corpus indexed and ready at {BENCH_DB}")


if __name__ == "__main__":
    main()
