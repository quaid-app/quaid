# Coverage Command Surface Cheap Arms

Use this when a Rust CLI is blocked just below a coverage gate and `main.rs` still contains many thin dispatch arms.

## Pattern

1. Read the live `target\llvm-cov-report.json` and list the still-missed command arms in `src\main.rs`.
2. Prefer one subprocess integration file over many scattered unit tests.
3. Batch only cheap commands whose real side effects are easy to seed and assert:
   - read/report commands (`stats`, `gaps`, `validate`, `compact`)
   - simple write commands (`tags`, `timeline-add`, `link-close`)
   - cheap import paths (`import`, `ingest`)
   - empty-surface commands (`embed --all` on an empty DB, `pipe` with one JSONL request)
4. Seed state through library helpers when that is cheaper than exercising extra CLI paths.
5. Re-run `cargo test --quiet -j 1`, then `cargo llvm-cov --lib --tests --summary-only --no-clean -j 1`, then refresh `target\llvm-cov-report.json`.

## Why it works

Dispatch arms are often the cheapest truthful coverage left late in a batch. One well-built subprocess suite can move both `main.rs` and command modules without inventing fake seams or bloating test count.
