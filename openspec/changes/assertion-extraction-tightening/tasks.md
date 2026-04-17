# Assertion Extraction Tightening — Implementation Checklist

**Scope:** Scope `extract_from_content` to `## Assertions` sections and frontmatter fields
only; add minimum object-length guard; document the structured assertion format.
Closes: #38

---

## Phase A — structured extraction in `src/core/assertions.rs`

- [ ] A.1 Add `extract_assertions_section(content: &str) -> &str` helper: scan `content`
  for a line matching `^## [Aa]ssertions` and return the substring from that heading to the
  next `^##` heading (or end of string). Return an empty string if no such section exists.

- [ ] A.2 Add `extract_from_frontmatter(frontmatter: &HashMap<String, String>) -> Vec<ExtractedAssertion>`
  helper: for each key in `["is_a", "works_at", "founded"]`, if the key is present and the
  value is non-empty, construct a Triple with the page's title/slug as subject (passed in),
  the key as predicate, and the value as object. Insert these with `evidence_text = "frontmatter"`.

- [ ] A.3 Modify `extract_from_content(content: &str) -> Vec<ExtractedAssertion>` to call
  `extract_assertions_section(content)` and run regex patterns against that scoped text only.
  Do not change the regex patterns themselves. If the section is empty, return an empty vec.

- [ ] A.4 Update `extract_assertions(page: &Page, conn: &Connection)` to call both
  `extract_from_frontmatter` and `extract_from_content`. The frontmatter map must be parsed
  from `page`'s frontmatter JSON field (already available). Merge results before inserting.

- [ ] A.5 Add minimum object-length guard in `collect_pattern_matches`: after constructing a
  `Triple`, discard it if `triple.object.trim().len() < 6`. Apply before `seen.insert`.

---

## Phase B — documentation

- [ ] B.1 Add a "Structured Assertions" section to `docs/spec.md` (or create
  `docs/assertions.md` if the spec is too large): document the `## Assertions` heading
  convention, the supported frontmatter fields (`is_a`, `works_at`, `founded`), and explain
  that general body text is not scanned.

- [ ] B.2 Update `gbrain check --help` text (in `src/commands/check.rs` or equivalent) to
  mention that assertions are extracted from structured zones only.

---

## Phase C — tests

- [ ] C.1 Add unit test: page with `## Assertions` section — verify correct triples extracted.
- [ ] C.2 Add unit test: page with no `## Assertions` section — verify zero triples extracted
  (the current false-positive case).
- [ ] C.3 Add unit test: page with frontmatter `is_a: researcher` — verify triple inserted
  via frontmatter path, not regex.
- [ ] C.4 Add unit test: object shorter than 6 chars (e.g., `is_a: it`) — verify discarded.

---

## Phase D — verification

- [ ] D.1 Import the 789-page PARA test vault (or a representative subset) and run
  `gbrain check --all`. Confirm zero false-positive contradictions from prose text.
- [ ] D.2 Add a page with a proper `## Assertions` section and run `gbrain check --all`.
  Confirm the assertion is detected and participates in contradiction checking correctly.
