# Mom — SLM runtime revision

- **Timestamp:** 2026-05-05T06:49:17.593+08:00
- **Context:** Reviewer rework for `slm-extraction-and-correction` runtime slice after Nibbler and Professor rejected Fry batch `2984150`.
- **Decision:** Treat first lazy-load failures as terminal for the in-memory SLM runtime, not just generation panics. `LazySlmRunner` now runtime-disables and fails closed after any initial load panic or verified local-cache/model-construction failure, while generation panics still disable the runtime the same way.
- **Why:** The first-use seam is the real crash boundary. If lazy load fails and the runtime keeps retrying, extraction keeps walking back into the same broken cache or constructor state and the daemon never reaches a stable fail-closed posture.
- **Test posture:** Guard all `QUAID_MODEL_CACHE_DIR` mutating SLM tests with a per-process mutex so targeted runtime tests stay deterministic under Rust's parallel test scheduler.
