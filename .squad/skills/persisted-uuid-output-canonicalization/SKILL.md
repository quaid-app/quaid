# Persisted UUID Output Canonicalization

Use this when a page/resource stores its stable identity outside agent-editable frontmatter or user-authored metadata.

## Pattern

1. Resolve the persisted identity from the durable store (`pages.uuid`, database id, canonical slug source of truth).
2. Before returning JSON or markdown to callers, remove any stale legacy alias fields from the metadata map.
3. Re-inject the canonical identity field into the emitted metadata/frontmatter payload.
4. Test the update path where the caller omits the identity field on write; the subsequent read must still surface the persisted identity.

## Why

Sparse frontmatter updates are allowed to omit identity fields. If read paths echo the stored frontmatter map verbatim, the resource keeps its real identity in the database but callers see a lie.

## Quaid fit

- `pages.uuid` is the durable source of truth for page identity.
- `quaid_id`/legacy `memory_id` inside frontmatter are presentation fields and must be canonicalized on read output.
- `memory_get` and JSON-oriented read surfaces should match `render_page()` behavior.
