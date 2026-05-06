# fact-resolution Specification

## Purpose
TBD - created by archiving change slm-extraction-and-correction. Update Purpose after archive.
## Requirements
### Requirement: Resolution applies dedup, supersede, coexist, or refusal per fact
For each parsed fact `F` produced by the SLM, the system SHALL perform a resolution step before writing. Resolution SHALL: (1) look up existing **head** pages where `kind = F.kind AND <type_key> = F.<type_key>` (the type key is `about` for `preference`/`fact`, `chose` for `decision`, `what` for `action_item`); (2) apply the candidate-count and embedding-evidence guardrails below; and (3) only when there is exactly one matching head and trustworthy semantic embedding evidence, compute the prose-embedding cosine between `F.summary` and that candidate head's body using the existing embedding pipeline.

- **Coexist (no match)**: no head shares the key → write `F` as a fresh head.
- **Typed ambiguity refusal**: if multiple heads share the key, resolution SHALL fail closed with an `AmbiguousMatchingHeads` error and write nothing. The "highest cosine wins" heuristic is NOT the policy in this slice. A future reviewed policy must explicitly land before multi-head disambiguation is permitted.
- **Typed evidence refusal**: if exactly one head shares the key but semantic embeddings are unavailable or hash-shim-only, resolution SHALL fail closed with an `UntrustworthyEmbeddingEvidence` error for that non-zero candidate set. This prevents pseudo-cosines from producing bad history.
- **Dedup**: with exactly one matching head and trustworthy semantic evidence, cosine > 0.92 → drop `F` entirely; do not write a new page.
- **Supersede**: with exactly one matching head and trustworthy semantic evidence, cosine in `[0.4, 0.92]` against an existing head `H` → write `F` as a new head with frontmatter `supersedes: <H.slug>`; the existing supersede chain machinery (delivered by `add-only-supersede-chain`) updates `H.superseded_by`.
- **Coexist (key match, low similarity)**: with exactly one matching head and trustworthy semantic evidence, cosine < 0.4 → write `F` as a fresh head; the shared key is incidental.

The cosine thresholds SHALL be configurable via `fact_resolution.dedup_cosine_min` (default `0.92`) and `fact_resolution.supersede_cosine_min` (default `0.4`). Resolution SHALL run inside a single immediate transaction for head lookup and resolution-time file-decision work only; the later watcher-driven ingest that inserts the page row and mutates `superseded_by` happens in a separate transaction and is not reserved by the resolution transaction.

#### Scenario: Near-duplicate fact is dropped
- **WHEN** F is `{kind: preference, about: programming-language, summary: "Matt prefers Rust"}` and an existing head has `summary: "User prefers Rust"` with cosine 0.95 to F
- **THEN** no new page is written, no supersede chain mutation occurs, and the existing head is unchanged

#### Scenario: Same key with mid-range similarity supersedes
- **WHEN** F is `{kind: preference, about: programming-language, summary: "Matt has switched to Zig"}` and an existing head has `summary: "Matt prefers Rust"` with cosine 0.55 to F
- **THEN** F is written as a new page with `supersedes: <existing-head-slug>`, and the existing head's `superseded_by` is updated to the new page's id

#### Scenario: Same key with low similarity coexists
- **WHEN** F is `{kind: preference, about: programming-language, summary: "Matt knows JavaScript well"}` and an existing head has `summary: "Matt prefers Rust for systems work"` with cosine 0.3 to F
- **THEN** F is written as a fresh head, no supersede chain change occurs, and both pages remain heads of their respective chains under the same `about` key

#### Scenario: No matching head means coexist
- **WHEN** F is `{kind: preference, about: editor, summary: "Matt uses Helix"}` and no existing head has `kind=preference, about=editor`
- **THEN** F is written as a fresh head with `superseded_by IS NULL` and `supersedes: null`

#### Scenario: Multi-match same key is refused (ambiguous head set)
- **WHEN** F is `{kind: fact, about: location, summary: "Matt lives in Tokyo"}` and three existing heads share `kind=fact, about=location`
- **THEN** resolution returns an `AmbiguousMatchingHeads` error; no file is written; all three heads are unchanged

#### Scenario: Resolution refused when embeddings are unavailable
- **WHEN** F is `{kind: preference, about: programming-language, summary: "Matt uses Zig"}` and one existing head shares the key, but the embedding backend has fallen back to hash-shim
- **THEN** resolution returns an `UntrustworthyEmbeddingEvidence` error; no file is written; the existing head is unchanged

### Requirement: Resolution uses head pages only; non-head pages are ignored
The head-lookup query in resolution SHALL filter to `superseded_by IS NULL`. Non-head (historical) pages SHALL NOT be candidates for dedup, supersede, coexist, or ambiguity checks. This ensures that a fact correction does not erroneously chain through a long-superseded ancestor.

#### Scenario: Historical pages are not candidates
- **WHEN** F is `{kind: preference, about: language, summary: "Matt prefers Rust"}` and the page graph contains heads `[H_current]` plus a historical chain `[H_old1 → H_old2 → H_current]`
- **THEN** resolution evaluates F against `H_current` only; `H_old1` and `H_old2` are ignored

### Requirement: Resolution writes via the vault, not directly to the database
After resolution decides on dedup / supersede / coexist / refusal, the write step SHALL produce a markdown file at the canonical path (`<vault>/extracted/<type-plural>/<slug>.md`) only for the supersede/coexist cases and SHALL rely on the Phase 4 vault watcher to ingest it as a page row. The supersede frontmatter (`supersedes: <prior_slug>`) SHALL be set on the file's frontmatter; the existing page-write code path (delivered by `add-only-supersede-chain`) SHALL handle the atomic two-end update of `superseded_by` on the prior head during that later ingest step.

#### Scenario: Supersede write produces a file with supersedes frontmatter
- **WHEN** resolution decides F supersedes head H
- **THEN** a new markdown file is written under `<vault>/extracted/<type-plural>/<F.slug>.md` whose frontmatter contains `supersedes: <H.slug>`, and the existing page-write logic (proposal #1) atomically inserts F's page row and updates H's `superseded_by`

#### Scenario: Dedup decision writes nothing
- **WHEN** resolution decides F is a duplicate of head H
- **THEN** no file is written, no page row is inserted, no chain mutation occurs, and H's `extracted_at` is not modified

### Requirement: Resolution is idempotent under re-extraction
For a deterministic SLM output (or a stable fact set produced by `quaid extract --force`), running resolution twice SHALL produce the same final supersede chain. Specifically: if a window is extracted, then `--force` re-extracts the same window from cursor 0, the resulting fact pages SHALL form an equivalent chain (same heads, same supersede relationships), modulo SLM nondeterminism in the prose body. The test for equivalence SHALL be on the structured frontmatter keys and the chain shape, not byte-equal prose.

#### Scenario: `--force` re-extraction yields an equivalent chain
- **WHEN** a session is fully extracted, then `quaid extract <session> --force` re-runs extraction from cursor 0 with the same model
- **THEN** the resulting set of head pages for each `(kind, type_key)` partition matches the prior set in number and structured key values; supersede chains have the same length; existing facts that match the new extraction outputs are de-duplicated rather than re-inserted as duplicates

