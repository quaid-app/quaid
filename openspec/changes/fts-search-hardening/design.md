## Context

`src/core/fts.rs` already contains `sanitize_fts_query`, and `src/core/search.rs` already uses it
for the hybrid search path. The benchmark regressions exist because the explicit interfaces
(`src/commands/search.rs` and `src/mcp/server.rs` `brain_search`) still call raw `search_fts`
directly, so FTS5 parser errors escape to the CLI and MCP layers.

That creates an architectural mismatch: one search surface behaves like a natural-language
product feature while the other behaves like an expert-only SQL-adjacent primitive. The release
problem is not that FTS5 cannot search these pages; it is that the user-facing entry points are
wired to the wrong abstraction and lack benchmark-visible regression coverage.

## Goals / Non-Goals

**Goals:**
- Give `gbrain search` and MCP `brain_search` the same natural-language-safe behavior for the
  issue #52 and #53 input class.
- Centralize the interface hardening logic so CLI and MCP do not drift again.
- Preserve a low-level raw FTS execution primitive for internal callers and focused unit tests.
- Add release-gate validation at the interface level, not just around a sanitizer helper.

**Non-Goals:**
- Building a new semantic recall or model-quality lane for benchmark section §4.
- Promising a separate contradiction algorithm project for issue #55 in this change.
- Adding a new public "expert raw FTS" CLI mode in v0.9.4.
- Reworking FTS ranking, tokenizer choice, or BM25 scoring.

## Decisions

### 1. Harden the explicit search interfaces with one shared wrapper

**Decision:** Introduce a shared wrapper around raw FTS execution for the explicit search
interfaces. The wrapper owns three steps: trim user input, apply the existing natural-language
sanitization policy, and return an empty success result when nothing searchable remains.

**Rationale:** The current bug is interface inconsistency, so the fix should live at the
interface boundary. One shared wrapper is simpler to review and maintain than duplicating
sanitization policy in `search.rs` and `server.rs`.

**Rejected alternative:** sanitize separately in the CLI and MCP handlers. Rejected because it
duplicates benchmark-critical policy and invites future drift.

### 2. Keep `search_fts` as the raw execution primitive

**Decision:** Leave `search_fts` itself as the low-level FTS5 executor and move the new contract
to a higher-level helper used by CLI and MCP.

**Rationale:** The repo already documents and tests `search_fts` as the exact-Fts primitive.
Changing its semantics would blur the boundary between raw engine behavior and product-facing
behavior, and it would make low-level FTS-focused tests harder to reason about.

**Rejected alternative:** make `search_fts` always sanitize. Rejected because it silently removes
raw FTS semantics from internal callers and obscures where user-facing policy actually lives.

### 3. Fail soft on empty-after-sanitize input

**Decision:** If a query becomes empty after sanitization (for example `???***`), the explicit
search surfaces should succeed with an empty result set rather than returning an error.

**Rationale:** This is the least surprising product behavior and matches the release goal of
eliminating crash/parse-error failures for natural-language input. It also keeps CLI text output
and MCP JSON output deterministic.

### 4. Put regression coverage where the benchmark failed

**Decision:** Add regressions at the command/MCP layer and keep the benchmark rerun as an
explicit completion gate for the lane.

**Rationale:** The current unit tests prove the sanitizer exists, but they did not stop a
release where the user-facing command path still crashed. The new tests must execute the same
surfaces Doug exercised.

## Risks / Trade-offs

- **User-facing FTS syntax becomes less expressive** → Mitigation: keep raw `search_fts` intact
  internally and document the CLI/MCP contract as natural-language-safe rather than expert FTS.
- **Wrapper drift between CLI and MCP** → Mitigation: one shared helper, not two copies.
- **Sanitization may reduce token fidelity for dotted versions** → Mitigation: accept token split
  as the safe v0.9.4 behavior and judge success by "valid results/no crash" rather than exact
  punctuation preservation.

## Migration Plan

- No schema changes and no data migration.
- Land the shared wrapper and interface regressions first.
- Rerun the benchmark validation commands for the FTS slice before closing the lane.
- If rerun evidence shows residual failures outside #52/#53, open a follow-on lane rather than
  widening this one opportunistically.

## Open Questions

- Whether a future release should expose an explicitly documented raw-FTS expert mode is
  deferred; it should not be folded into the v0.9.4 hardening lane.
