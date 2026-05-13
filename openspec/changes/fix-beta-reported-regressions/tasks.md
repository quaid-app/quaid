## 1. Reproduce and Characterize

- [ ] 1.1 Add focused fixtures/tests for scalar `related` frontmatter import and export.
- [ ] 1.2 Add focused fixtures/tests for PARA type inference with graph frontmatter present.
- [ ] 1.3 Add or update shutdown regression coverage for `quaid serve` SIGTERM cleanup.
- [ ] 1.4 Add embed batching coverage for CLI validation and partial rerun/idempotence.

## 2. Frontmatter, PARA, and Round-trip Fixes

- [ ] 2.1 Update graph/frontmatter relationship parsing to coerce scalar `related` to a single-item list while preserving list behavior.
- [ ] 2.2 Ensure invalid optional graph metadata is isolated to diagnostics and does not abort page ingest/export when the page is otherwise valid.
- [ ] 2.3 Restore page type derivation so explicit and inferred PARA types survive collection ingest and graph/tag sync.
- [ ] 2.4 Verify round-trip export writes every successfully ingested page in the regression fixtures.

## 3. Runtime and Embedding Fixes

- [ ] 3.1 Update MCP/serve shutdown to cancel owned runtime workers and wait for owned children without touching unrelated processes.
- [ ] 3.2 Add `quaid embed --batch-size N` validation and conservative default batching.
- [ ] 3.3 Refactor embedding execution to process pending pages in bounded batches and preserve rerun idempotence.

## 4. Release and Issue Closure

- [ ] 4.1 Update changelog/docs for v0.22.1 beta regression fixes.
- [ ] 4.2 Run focused tests for changed areas, then full release validation.
- [ ] 4.3 Open and merge a PR for the OpenSpec and implementation.
- [ ] 4.4 Tag and publish v0.22.1, verify release assets, and comment/close issues #190-#195 as appropriate.
