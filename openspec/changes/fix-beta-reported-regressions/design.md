## Context

The v0.22.0 knowledge graph release introduced structured frontmatter parsing and additional runtime work during ingest/search. A beta DAB v1.0 run reported regressions in import classification, import/export completeness, server shutdown cleanup, and embed stability. The patch must restore v0.21.0-compatible behavior while preserving v0.22.0 graph capabilities and default-off graph retrieval.

## Goals / Non-Goals

**Goals:**
- Restore PARA type distribution during collection ingest and round-trip export.
- Accept recoverable scalar frontmatter forms used by real-world markdown, especially `related: some-slug`.
- Make MCP shutdown deterministic for Quaid-owned worker processes after SIGTERM.
- Bound embedding memory use for medium corpora with a conservative default and an override flag.
- Ship as a patch release without a schema reset.

**Non-Goals:**
- Improve DAB §4 semantic/hybrid quality, BGE-small paraphrase behavior, or FTS contamination beyond the filed beta issues.
- Turn graph retrieval expansion on by default.
- Add durable `entity_pattern` link provenance beyond the v0.22.0 assertions-only scope.
- Change MCP tool names, schemas, or response shapes.

## Decisions

1. **Treat frontmatter coercion at the parsing edge.** Normalize known relationship fields (`parent`, `children`, `related`) when syncing graph edges instead of rejecting the whole page. This keeps storage structured while making ingest tolerant of common scalar shorthand. Alternative considered: reject invalid fields and skip only graph autowire; rejected because collection attach still fails and round-trip completeness remains broken.

2. **Keep page type inference independent from graph autowire.** Type derivation must read explicit frontmatter type first, then path/content heuristics, and graph processing must not overwrite a derived PARA type with the fallback `concept`. Alternative considered: special-case DAB paths only; rejected because the regression is a general import-classification bug.

3. **Prefer graceful runtime cancellation over process-name cleanup.** Shutdown should notify runtime workers and wait briefly for owned children/threads before process exit. Tests should assert process cleanup by PID where possible rather than using broad `pkill`-style behavior. Alternative considered: force-kill any `quaid` process; rejected because it risks terminating unrelated user sessions.

4. **Batch embed work at the command/runtime layer.** `quaid embed` should process pages in bounded chunks and expose `--batch-size`, defaulting conservatively. Alternative considered: rely on OS retry behavior; rejected because first-run SIGKILL is user-visible data-pipeline failure.

## Risks / Trade-offs

- Recovering scalar frontmatter could hide malformed data → Limit coercion to unambiguous string/list relationship fields and keep invalid complex values reported consistently.
- Serialized or smaller embed batches may reduce throughput → Prefer successful bounded memory behavior over peak throughput; users can raise `--batch-size`.
- Shutdown cleanup may expose pre-existing long-running worker assumptions → Use focused integration tests and preserve detached daemon semantics.
- Some DAB delta=9 files may fail for additional malformed-frontmatter reasons → Add tests around the known `related` failure and inspect any remaining import/export skips before release.

## Migration Plan

No database migration is planned. The patch changes ingest/runtime behavior and adds tests. Existing v10 databases remain valid; users rerun collection sync or embed commands to benefit from fixed behavior.

Rollback is a normal binary downgrade to v0.22.0, with the caveat that v0.22.0 retains the beta-reported bugs.

## Open Questions

- Whether issue #192 reproduces only for `quaid serve` stdio or also for daemon-hosted HTTP/SSE. Implementation should cover both Quaid-owned runtime shutdown paths where practical.
