#!/usr/bin/env bash
# Quaid mini-benchmark for rapid dev feedback
# Usage: ./scripts/mini-bench.sh [--no-build]
#
# Scores FTS and semantic search across 20 hand-crafted queries.
# Outputs a visual progress bar with per-query pass/fail.
# Exits 1 if total < 8/20 (regression gate).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(dirname "$SCRIPT_DIR")"
BENCH_DB="/tmp/quaid-mini-bench.db"
PREV_SCORE_FILE="/tmp/quaid-mini-bench-prev.txt"
QUAID_BIN="$REPO_DIR/target/release/quaid"
CORPUS_DIR="/tmp/quaid-bench-corpus"

cd "$REPO_DIR"

# Build unless --no-build
if [[ "${1:-}" != "--no-build" ]]; then
  echo "Building Quaid (release)..."
  export PATH="$HOME/.cargo/bin:$PATH"
  cargo build --release 2>&1 | tail -3
  echo ""
fi

# Verify binary exists
if [[ ! -f "$QUAID_BIN" ]]; then
  echo "ERROR: Binary not found at $QUAID_BIN"
  echo "  Run: cargo build --release"
  exit 1
fi

# Generate corpus if missing
if [[ ! -d "$CORPUS_DIR" ]] || [[ -z "$(ls -A "$CORPUS_DIR" 2>/dev/null)" ]]; then
  echo "Generating corpus at $CORPUS_DIR..."
  python3 "$REPO_DIR/scripts/mini-bench-setup.py" --corpus-only
fi

# Setup DB if missing
if [[ ! -f "$BENCH_DB" ]]; then
  echo "First run: building corpus DB (one-time, ~60s)..."
  python3 "$REPO_DIR/scripts/mini-bench-setup.py"
  echo ""
fi

# Run benchmark
python3 "$REPO_DIR/scripts/mini-bench-run.py" \
  --db "$BENCH_DB" \
  --quaid "$QUAID_BIN" \
  --prev "$PREV_SCORE_FILE"
