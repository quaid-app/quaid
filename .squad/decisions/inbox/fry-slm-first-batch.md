# Fry — SLM first batch boundary

- Date: 2026-05-05
- Change: `slm-extraction-and-correction`

## Decision

Land the first truthful batch as the v9 schema/config reset only: `correction_sessions`, extraction/fact-resolution config defaults, schema-version bump, and the rejection/acceptance tests that prove fresh v9 bootstrap and fail-closed v8 reopen behavior.

## Why

- Every later SLM/control/worker slice depends on the persisted schema and defaults being stable first.
- The branch is already dirty in nearby conversation/runtime files, so keeping Batch 1 to schema + tests avoids widening into active seams before the base contract is locked.
- This keeps the branch moving toward v0.19.0 with a reviewable, low-blast-radius slice that future runtime/CLI work can build on.

## Follow-up

- Next batch should start at runtime/model lifecycle wiring (`2.*` / `3.*`) or the thinnest CLI plumbing that consumes the new defaults without broadening into worker/correction orchestration prematurely.
