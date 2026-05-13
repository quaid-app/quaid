#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(dirname "$SCRIPT_DIR")"
BENCH_DB="/tmp/quaid-mini-bench.db"
QUAID_BIN="$REPO_DIR/target/release/quaid"

cd "$REPO_DIR"

if [[ "${1:-}" != "--no-build" ]]; then
  echo "Building quaid..."
  cargo build --release 2>&1 | tail -2
fi

if [[ ! -f "$BENCH_DB" ]]; then
  echo "First run: setting up corpus and DB (~60s)..."
  node scripts/mini-bench-setup.mjs --quaid "$QUAID_BIN"
fi

node scripts/mini-bench-run.mjs --db "$BENCH_DB" --quaid "$QUAID_BIN"
