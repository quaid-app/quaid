---
name: gbrain-query
description: |
  Answer questions from the brain using FTS5 + semantic search + structured queries.
  Synthesize across multiple pages. Cite sources.
---

# Query Skill

## Commands

### Hybrid search (recommended)

```bash
gbrain query "who knows about fintech?"
gbrain query "who knows about fintech?" --wing people
gbrain query "AI agents" --limit 5 --json
```

Runs the four-layer search strategy and returns ranked results under a token budget.

### FTS5 keyword search

```bash
gbrain search "machine learning"
gbrain search "fundraising" --wing companies
gbrain search "infrastructure" --limit 10 --json
```

Pure BM25-ranked keyword search over `compiled_truth` + `timeline`.

### Exact page lookup

```bash
gbrain get people/alice
gbrain get people/alice --json
```

Direct slug lookup. Returns the full page content or JSON representation.

## Strategy: Four-layer search

1. **SMS (Exact-Match Short-Circuit)** — If the query matches a slug exactly or
   is wrapped in `[[slug]]`, return that page immediately. Skip all other layers.
2. **FTS5 full-text** — BM25-ranked keyword search across `compiled_truth` + `timeline`.
3. **Vector semantic** — Cosine similarity via BGE-small-en-v1.5 embeddings (384-dim).
   Currently running as SHA-256 hash placeholder until Candle model is wired.
4. **Set-union merge** — Combine FTS5 and vector result sets. Pages in either set are
   returned, ranked by normalised combined score (configurable: `set_union` or `rrf`).

## Token Budget

The `query` command applies a token budget to results:
- Results are returned in score order up to `--limit`
- Each result's output line counts against the budget
- If a result exceeds remaining budget, its summary is truncated
- Default budget: 4096 tokens, configurable via `--token-budget`

## JSON Output

All search commands support `--json` for structured output:
- `query --json`: array of SearchResult objects
- `search --json`: array of SearchResult objects
- `get --json`: full Page object
- `list --json`: array of page summary objects

## Merge Strategy Configuration

```bash
gbrain config set search_merge_strategy rrf    # reciprocal rank fusion
gbrain config set search_merge_strategy union  # set union (default)
```

