# Fry — Phase 1 Markdown Slice (T03)

**Date:** 2026-04-14
**Author:** Fry (Main Engineer)
**Scope:** `src/core/markdown.rs` — frontmatter, split, summary, render

---

## Decisions

### 1. Frontmatter renders with sorted keys

`render_page` emits frontmatter keys in alphabetical order. This makes output
deterministic and enables byte-exact round-trip for canonical input. Canonical
format is: unquoted YAML values, alphabetically sorted keys.

**Implication for Bender:** `roundtrip_raw.rs` fixtures must use alphabetically
sorted frontmatter keys to pass the byte-exact gate.

### 2. Timeline separator only emitted when timeline is non-empty

The design says "compiled_truth + `\n---\n` + timeline" but emitting `\n---\n`
for empty timelines adds spurious content that doesn't exist in the original.
`render_page` omits the separator when `page.timeline.is_empty()`. This still
round-trips correctly: `split_content` returns empty timeline when no `---`
line exists.

### 3. YAML parse failures degrade gracefully (no ParseError)

`parse_frontmatter` returns `(HashMap<String, String>, String)` — no `Result`.
Malformed YAML produces an empty map; the body is still extracted correctly.
This matches the spec signature and avoids error-handling complexity for a
pure-text operation. If stricter validation is needed later (e.g., `--validate-only`
mode in `gbrain import`), it should happen at the command layer, not in the
parsing core.

### 4. Non-scalar YAML values silently skipped

Sequences and mappings in frontmatter (e.g., `tags: [foo, bar]`) are dropped
by `parse_yaml_to_map`. The `HashMap<String, String>` contract only holds
scalars. Tags are stored separately in the `tags` table, not in frontmatter.

---

## Routing

- **Bender:** test fixtures for `roundtrip_raw` must use canonical format (sorted keys).
- **Professor:** no review needed for this slice — pure text parsing with no DB/search impact.
- **Leela:** T03 complete. Next dependency-available task: T04 (palace.rs).
