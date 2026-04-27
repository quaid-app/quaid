## Context

`src/core/assertions.rs` extracts subject-predicate-object triples from page content
using three regex patterns (is_a, works_at, founded). These run against the full
`compiled_truth` text of each page. The extracted triples are stored in the `assertions`
table and compared across pages by `check_assertions` to detect contradictions.

The `asserted_by` column distinguishes `'agent'` (heuristic extraction) from `'user'`
(manually inserted). This fix only affects the `'agent'` path.

---

## Decisions

### 1. Scope extraction to an explicit `## Assertions` section

**Decision:** Run regex extraction only against the content between a `## Assertions`
(or `## assertions`) heading and the next `##`-level heading. If no such section exists,
extract zero agent assertions.

**Rationale:** This is the minimal, backward-compatible change that eliminates false
positives. Users who want assertions extracted must add a dedicated section — this
creates a clear, teachable contract. It does not require a new syntax or schema change.

### 2. Frontmatter field extraction as tier 1

**Decision:** Before regex extraction, inspect `frontmatter` fields: if `is_a`, `works_at`,
or `founded` keys are present, insert them as typed triples directly (no regex needed).
These are trusted because they are explicitly authored.

**Rationale:** Frontmatter is structured data. A field `is_a: researcher` is unambiguously
an assertion. This path should be fast and noise-free.

### 3. Minimum object-length guard

**Decision:** Discard any regex-extracted triple whose `object` field is shorter than
6 characters or contains fewer than 1 whitespace-bounded word after trimming. Applied after
extraction, before insert.

**Rationale:** Noise matches like `is_a: it`, `is_a: the` slip through the current regex
and create trivially-false contradictions. The guard is simple and cheap.

### 4. No `[[is_a: X]]` inline syntax in this lane

**Decision:** Inline wikilink-style assertion syntax is out of scope. Deferred to a future
change (`inline-assertion-syntax`).

**Rationale:** The scope needed to fix the false-positive problem is narrow. Inline syntax
requires parser changes, documentation, and community discussion around the format. The
`## Assertions` section approach solves the immediate problem without that investment.

### 5. No migration for existing pages

**Decision:** No automatic re-extraction on upgrade. Existing pages that relied on prose
extraction will show zero agent assertions after this change. Users re-import or add explicit
`## Assertions` sections.

**Rationale:** A migration would require scanning all pages, re-extracting, and re-running
check — risky for large vaults. The change moves the system from "too noisy" to "too quiet"
which is a safer default. Users can restore precision by adding structured sections.

### 6. Semantic similarity gate for cross-page comparison (#55) — deferred pending rerun

**Decision:** Out of scope for this lane. Do not implement a cosine-similarity pre-filter
in the contradiction-detection path as part of this change.

**Rationale:** Kif's v0.9.4 triage directs the team to land extraction tightening first,
rerun Doug's benchmark corpus, and only open a new lane for the semantic gate if false
positives materially survive after tightening. Implementing a similarity gate before that
rerun risks solving a problem that no longer exists, adds complexity to the critical path,
and requires embedding infrastructure that may not be necessary. If the rerun shows the
gate is still needed, it will be proposed as a standalone lane (`assertion-similarity-gate`)
with its own spec and acceptance criteria.
