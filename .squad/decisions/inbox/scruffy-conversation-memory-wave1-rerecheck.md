# Scruffy — conversation-memory Wave 1 rerecheck

- **Timestamp:** 2026-05-04T07:22:12.881+08:00
- **Scope:** `conversation-memory-foundations` tasks 4.1-6.6 after commit `5c88104`
- **Decision:** Keep the Wave 1 truth note closed without adding more tests in this lane. The full Windows rerecheck still clears the requested floor (`cargo test -j 1` passes; `RUST_TEST_THREADS=1 cargo llvm-cov --lib --tests --summary-only -j 1` reports 90.01% total line coverage), and the three revision seams already have direct proof in the landed suite.
- **Why:** The new queue lease-token path is covered by stale-claim tests in `tests\\extraction_queue.rs`, the same-session cross-process serialization path is covered by the child-process lock test in `tests\\conversation_turn_capture.rs`, and the explicit metadata-fence path is covered by both round-trip and bare-trailing-JSON preservation tests in `src\\core\\conversation\\format.rs`. The remaining misses in those files are mostly config/error helpers and platform branches, so adding filler tests here would not make the task truth any more honest.
- **Coverage note:** This is a truthful Windows/stable rerecheck only. It does not claim nightly branch coverage, and it does not pretend to execute Unix-only lock behavior from this host.
