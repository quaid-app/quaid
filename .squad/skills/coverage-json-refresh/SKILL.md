# Coverage JSON Refresh

Use this after `cargo llvm-cov --summary-only` when you still need exact line movement per file.

## Pattern

1. Run the required measurement command first:
   `cargo llvm-cov --lib --tests --summary-only --no-clean -j 1`
2. Immediately refresh the detailed export without re-running tests:
   `cargo llvm-cov report --json --output-path target\\llvm-cov-report.json`
3. Diff the refreshed JSON against the prior missed-line set for just the files in your lane.
4. Use the summary command for the release gate number, and the refreshed JSON for exact line/file movement.

## Why

`--summary-only` gives the truthful gate number fast, but it does not by itself refresh the checked-in detailed line map used for tactical coverage work. The follow-up `report --json` step is the honest way to update `target\\llvm-cov-report.json` after the measurement run.
