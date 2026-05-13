## Why

Beta DAB v1.0 validation for v0.22.0 regressed from v0.21.0, with confirmed failures in PARA type inference, frontmatter import tolerance, round-trip export completeness, MCP shutdown cleanup, and first-run embedding memory pressure. These issues block confidence in the knowledge graph release and need a patch release before additional graph work proceeds.

## What Changes

- Restore PARA page type inference so collection ingest preserves/derives project, area, resource, archive, and concept classifications instead of collapsing most imported pages to `concept`.
- Make graph/frontmatter parsing tolerant of common scalar forms such as `related: some-slug`, coercing them to single-item lists instead of aborting collection attach.
- Preserve import/export round-trip behavior for files with non-canonical but recoverable frontmatter, including exporting every ingested page.
- Ensure `quaid serve` and MCP runtime shutdown terminate child/worker processes cleanly on SIGTERM.
- Add bounded embedding batch behavior so `quaid embed` can process a 350-page corpus without first-run OOM/SIGKILL behavior.
- Release v0.22.1 with documentation/changelog updates and close addressed beta issues.

## Capabilities

### New Capabilities
- `mcp-server-shutdown`: MCP server shutdown and signal handling terminate all Quaid-owned runtime workers/processes.
- `embedding-batch-processing`: Embedding commands process pages in bounded batches with an operator-visible batch-size control.

### Modified Capabilities
- `collections`: Collection import and page classification preserve PARA type inference across graph/frontmatter processing.
- `frontmatter-link-autowiring`: Frontmatter graph fields accept scalar shorthand where unambiguous and do not reject otherwise importable pages.
- `vault-sync`: Round-trip export/import remains complete for recoverable frontmatter and collection state.

## Impact

- Affected GitHub issues: #190 parent benchmark report; fixes #191, #192, #193, #194, and #195.
- Affected code: markdown/frontmatter parsing, collection ingest/reconciliation, page type derivation, graph autowire, MCP/daemon shutdown, embedding command/runtime, release docs.
- No schema version bump is expected unless investigation finds a persisted-state bug that cannot be repaired behaviorally.
- No breaking CLI or MCP API changes are intended; any new embed batch flag must be backward-compatible with current defaults.
