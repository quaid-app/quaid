# scruffy — History Summary

**Last Summarized:** 2026-05-04T00:00:30Z

**Active Work:** Conversation-memory improvements (truth-repair, slice validation)

**Status:** Contributing to collaborative batch session

_Archived history available in history-archive.md_

## Learnings

- 2026-05-04T07:22:12.881+08:00 — Supersede/retrieval coverage needs branch-specific proofs beyond the happy-path integration: exact-slug head filtering, progressive expansion filtering, and graph supersede edges each need their own test seam, or coverage claims overstate the slice.
- 2026-05-04T07:22:12.881+08:00 — On this Windows/stable lane, the honest post-`d98e010` check is `cargo test -j 1` plus `RUST_TEST_THREADS=1 cargo llvm-cov --lib --tests --summary-only -j 1`: repo line coverage still clears 90%, but llvm-cov branch mode is nightly-only and the deterministic supersede-race proof in `src\commands\put.rs` stays Unix-gated, so report that limitation instead of overstating branch-proof coverage.
- 2026-05-04T07:22:12.881+08:00 — For conversation-memory foundations, coverage moves fastest when the proof splits three ways: parser/render round-trips in-module, file-append durability/day-rollover in a focused integration test, and queue semantics in a separate SQLite-backed integration test. That keeps the changed seams honest without dragging MCP wiring into the slice before it exists.
- 2026-05-04T07:22:12.881+08:00 — After Mom's Wave 1 revision commit `5c88104`, the honest Windows rerecheck is still `cargo test -j 1` plus `RUST_TEST_THREADS=1 cargo llvm-cov --lib --tests --summary-only -j 1`: repo-wide line coverage holds at 90.01%, and the new lease-attempt, cross-process append lock, and explicit `json turn-metadata` fence seams already have direct tests, so no extra padding tests should be invented.
