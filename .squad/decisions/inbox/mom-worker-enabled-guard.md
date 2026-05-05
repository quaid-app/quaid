# Mom — worker enable guard

- **Date:** 2026-05-05
- **Context:** `slm-extraction-and-correction` worker-loop revision for spec item `5.2`
- **Decision:** The worker's `claim_next_job` seam owns the `extraction.enabled` gate and must return `None` before dequeuing when extraction is disabled; pending rows stay untouched until extraction is re-enabled.
- **Why:** Letting the worker claim first and fail later mutates queue state while the system is explicitly disabled, which makes the idle/disabled contract dishonest and burns retries for a state that should be a pure no-op.
