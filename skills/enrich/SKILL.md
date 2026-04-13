---
name: gbrain-enrich
description: |
  Enrich brain pages with external data from Crustdata, Exa, Partiful, etc.
---

# Enrich Skill

> Stub — full content to be authored in Phase 3 implementation.

## Sources

- **Crustdata** — company + person professional data
- **Exa** — web search and content extraction
- **Partiful** — event and social data

## Storage

Enrichment data stored in `raw_data` table (one row per source per page).
Structured facts extracted into `compiled_truth` and `assertions`.

## TODO

- [ ] Crustdata integration patterns
- [ ] Exa query templates
- [ ] Enrichment → assertion extraction rules
- [ ] Conflict resolution when enrichment contradicts existing compiled_truth
