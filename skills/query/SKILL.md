---
name: gbrain-query
description: |
  Answer questions from the brain using FTS5 + semantic search + structured queries.
  Synthesize across multiple pages. Cite sources.
---

# Query Skill

> Stub — full content to be authored in Phase 1 implementation.

## Strategy: Four-layer search

1. **SMS (Exact-Match Short-Circuit)** — If the query matches a slug exactly, return that page immediately. Skip all other layers.
2. **FTS5 full-text** — BM25-ranked keyword search across compiled_truth + timeline.
3. **Vector semantic** — cosine similarity via BGE-small-en-v1.5 embeddings (384-dim).
4. **Set-union merge** — Combine FTS5 and vector result sets. Return pages that appear in either set, ranked by best position.

## Progressive Retrieval

Expand results lazily under a token budget:
1. Return summaries first
2. If budget allows, expand to relevant sections
3. If budget allows, expand to full pages

## TODO

- [ ] Full four-layer search workflow
- [ ] Token budget management
- [ ] Citation format
- [ ] Synthesis heuristics
