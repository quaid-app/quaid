# Orchestration: Professor — Phase 1 Search/Embed/Query Review (REJECTION)

**Timestamp:** 2026-04-14T04:56:03Z  
**Coordinator:** Scribe  
**Agent:** Professor (Code Review)  
**Directive:** macro88 (Copilot v0.9.1 Team Mode)

## Mandate

Code review of Fry's T14–T19 artifact (`2d5f710`) for Phase 1 search/embed/query surface.  
Validate against contract specification, semantic completeness, and test compilation.

## Status

**REJECTION FOR LANDING**

Three blocking findings prevent landing. The FTS path is on-spec, but semantic search surface
is misleading and embed CLI contract drifts. Tree fails `cargo test` compilation.

## Blocking Findings

### 1. Inference Shim Instead of Candle

`src/core/inference.rs` promises BGE-small-en-v1.5 embeddings in public API but substitutes
a SHA-256 hash-based placeholder. No `candle_*` wiring, no embedded model weights, no `online-model`
feature gate implemented. This creates false semantic guarantees: vector similarity scores are
hash-proximity, not semantic distance. BEIR benchmarks against this will be meaningless.

**Action required:** Implement Candle/BGE or explicitly defer semantic search to Phase 2 with
documented placeholders.

### 2. Embed CLI Mixed-Mode Allowed

Contract: `gbrain embed [SLUG | --all | --stale]` (mutually exclusive modes).  
Current: Parsing allows mixed modes (`SLUG` + `--all`, `SLUG` + `--stale`). Implementation silently
privileges slug path instead of rejecting invalid combinations. Also, `--all` re-embeds everything
instead of skipping unchanged content per spec.

**Action required:** Add CLI validation to reject mode mixtures; implement `--all` skip-unchanged.

### 3. Tests Do Not Compile

`embed::run` signature changed to `(db, slug, all, stale)` (4 args).  
Test callsites still use old 3-arg form. Result: `cargo test` fails before review can complete.

**Action required:** Update all test callsites to new signature.

## Non-Blocking Note

`src/commands/query.rs` exposes `--depth` flag but ignores it (deferred to Phase 2).  
Help text should clarify this or remove the flag from Phase 1 surface.

## Outcome

Rejection is scoped to semantic-search truthfulness, embed CLI contract integrity, and build
breakage. FTS implementation itself is acceptable. Recommend Fry address blockers and resubmit,
or defer semantic search blocker to Phase 2 if time-bound.

**Next step:** Leela revision cycle initiated.
