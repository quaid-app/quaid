---
name: cli-slug-parity
description: Keep CLI collection-aware slug routing aligned with MCP by resolving once and canonicalizing page references at the output boundary.
---

# CLI Slug Parity

Use this when a CLI command needs to match MCP collection-aware routing without widening into new filters or tools.

## Pattern

1. **Resolve once at the boundary**
   - Accept either bare `slug` or explicit `<collection>::<slug>`.
   - Use the shared collection-aware resolver with the same `OpKind` classification as the MCP surface.

2. **Carry keyed identity internally**
   - After resolution, do not fall back to bare-slug SQL lookups.
   - Use `(collection_id, slug)` or `page_id` for every subsequent read/write/query step.

3. **Canonicalize only when returning page references**
   - Any CLI output that references a page should print `<collection>::<slug>`.
   - This includes JSON rows, text summaries, graph edges/nodes, contradiction reports, and rendered page frontmatter/output.

4. **Test the seam end-to-end**
   - Spawn the real CLI where practical.
   - Cover:
     - explicit `<collection>::<slug>` success
     - ambiguous bare-slug failure
     - canonical page references in output
     - at least one mutating CLI surface and one read/reporting surface

## Guardrails

- Do **not** widen into deferred collection filters/defaults just because results now carry canonical slugs.
- If a command processes many pages (`check --all`, graph traversal, backlinks, etc.), make sure internal iteration is keyed by collection/page identity rather than raw slug text.
- Canonicalize negative paths too once resolution has happened. A no-op/error message such as `unlink` finding no matching edge is still a page-referencing CLI output and should not fall back to raw user input.
- If a query-like command has an exact-slug fast path, ambiguous bare slugs must fail closed there too. Do **not** collapse ambiguity into “no result” or generic search fallback once the input is being treated as a page address.
