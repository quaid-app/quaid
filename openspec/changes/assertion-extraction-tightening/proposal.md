---
id: assertion-extraction-tightening
title: "check --all: tighten assertion extraction to eliminate false-positive contradictions"
status: proposed
type: bug
owner: professor
reviewers: [leela, nibbler]
created: 2026-04-19
depends_on: p3-skills-benchmarks
closes: ["#38"]
---

# check --all: tighten assertion extraction to eliminate false-positive contradictions

## Why

`gbrain check --all` produces a flood of false-positive contradiction reports on real vaults.
The root cause is in `src/core/assertions.rs`: `extract_from_content` runs broad regex patterns
(is_a, works_at, founded) across the full `compiled_truth` section of every page. Any prose
sentence that happens to match — analysis text, summaries, quoted sources — becomes an
`is_a` or `works_at` assertion and gets compared against assertions from every other page.

Beta tester doug-aillm (issue #38) reproduced this with a 789-page Obsidian vault:
`check --all` produced 10+ false conflicts all tracing back to a single research-notes page
whose prose happened to match the is_a pattern in many different ways. The output was
completely unusable.

The fix is structural: only extract assertions from explicitly-structured content, not arbitrary
body prose.

## What Changes

### 1. `src/core/assertions.rs` — scope extraction to structured zones only

Assertion extraction moves from "scan all of `compiled_truth`" to "scan only content inside
an explicit `## Assertions` section (if present) and structured frontmatter fields."

Extraction rules (in priority order):

1. **Frontmatter fields** (highest trust): `is_a: founder`, `works_at: Acme Corp`,
   `founded: Brain Co` — parse as typed assertions without regex.
2. **`## Assertions` section** (explicit opt-in): if the page's `compiled_truth` contains a
   `## Assertions` heading, extract only from the content between that heading and the next
   `##`-level heading. Regex patterns continue to apply within this scoped zone.
3. **Inline syntax** (future): `[[is_a: X]]` wikilink-style markers — reserved for a
   future change; do not implement in this lane.

If neither frontmatter fields nor an `## Assertions` section is present, extract zero
assertions. Do not fall back to scanning general body text.

### 2. `src/core/assertions.rs` — minimum object-length guard

Add a guard: any regex-extracted object (the `object` field of the triple) that is fewer than
3 tokens (whitespace-split words) or shorter than 6 characters is discarded. This filters out
single-word noise matches like `is_a: "it"` or `is_a: "the"`.

### 3. `src/schema.sql` — no schema changes

The `assertions` table schema is unchanged. The `asserted_by` column already distinguishes
`'agent'` (heuristic) from `'user'` (manual). This fix only affects the `'agent'` extraction
path; manual assertions are unaffected.

### 4. Docs — document the Assertions section format

- `docs/spec.md` or equivalent: add a "Structured Assertions" section documenting:
  - The `## Assertions` heading convention.
  - Supported frontmatter assertion fields (`is_a`, `works_at`, `founded`).
  - The decision to not extract from general body text.
- `README.md` or `gbrain check --help`: update the check command description to mention that
  assertions are extracted from structured zones only.

## Non-Goals

- Adding new assertion predicates — this change only fixes extraction scoping.
- Implementing `[[is_a: X]]` inline syntax — deferred to a future lane.
- Changing contradiction-matching logic — only the extraction side is affected.
- Re-extracting assertions for already-imported pages automatically — users will need to
  re-run `gbrain check` or re-import; no migration script is required.

## Impact

- `src/core/assertions.rs`: new `extract_assertions_section` helper; modified
  `extract_from_content` to call it instead of scanning full content; new minimum-length guard.
- `docs/spec.md` (or equivalent): assertion format documentation added.
- Existing vault users will see fewer contradictions reported after this change. Pages that
  relied on prose extraction for assertions will need to add an explicit `## Assertions`
  section to retain them.
