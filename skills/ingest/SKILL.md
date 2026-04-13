---
name: gbrain-ingest
description: |
  Ingest meeting notes, articles, documents, and conversations into GigaBrain.
  Handles Tier 1-4 processing: immediate memory, recent consolidation, long-term knowledge, and temporal archive.
---

# Ingest Skill

> Stub — full content to be authored in Phase 1 implementation.

## Overview

The ingest skill processes raw source documents into structured brain pages.
It handles novelty detection, entity extraction, link creation, assertion logging,
and idempotent write operations.

## Tiers

1. **Tier 1 (Immediate)** — Write directly as meeting/conversation pages
2. **Tier 2 (Recent)** — Consolidate into entity compiled_truth sections
3. **Tier 3 (Long-term)** — Cross-link entities, detect contradictions
4. **Tier 4 (Archive)** — Timeline entries, temporal sub-chunking

## Source Attribution Format

All ingested pages should include source attribution in frontmatter:
```yaml
sources:
  - authority: primary|secondary|tertiary
    type: meeting|article|conversation|manual
    ref: "meeting/2024-03-01" | "https://..." | "file/..."
    date: YYYY-MM-DD
```

## Filing Disambiguation

When the same entity could go in multiple wings, prefer the wing
that matches the slug prefix (e.g., `people/` → `people` wing).

## TODO

- [ ] Full ingest workflow (Tiers 1-4)
- [ ] Novelty check integration
- [ ] Assertion extraction rules
- [ ] Contradiction detection hooks
- [ ] Source attribution normalization
