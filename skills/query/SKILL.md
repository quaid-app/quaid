---
name: quaid-query
description: |
  Answer questions from the brain using FTS5 + semantic search + structured queries.
  Synthesize across multiple pages. Cite sources.
---

# Query Skill

## Commands

### Hybrid search (recommended)

```bash
quaid query "who knows about fintech?"
quaid query "who knows about fintech?" --wing people
quaid query "AI agents" --limit 5 --json
```

Runs the four-layer search strategy and returns ranked results under a token budget.

### FTS5 keyword search

```bash
quaid search "machine learning"
quaid search "fundraising" --wing companies
quaid search "infrastructure" --limit 10 --json
```

Pure BM25-ranked keyword search over `compiled_truth` + `timeline`.

### Exact page lookup

```bash
quaid get people/alice
quaid get people/alice --json
```

Direct slug lookup. Returns the full page content or JSON representation.

## Strategy: Four-layer search

1. **SMS (Exact-Match Short-Circuit)** — If the query matches a slug exactly or
   is wrapped in `[[slug]]`, return that page immediately. Skip all other layers.
2. **FTS5 full-text** — BM25-ranked keyword search across `compiled_truth` + `timeline`.
3. **Vector semantic** — Cosine similarity via the brain's configured embedding model.
   Default is BGE-small-en-v1.5 (384-dim); online builds may also use `base`, `large`,
   `m3`, or another Hugging Face model ID selected via `QUAID_MODEL` / `--model`.
   Currently falls back to a SHA-256 hash placeholder when Candle weights are unavailable.
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
quaid config set search_merge_strategy rrf    # reciprocal rank fusion
quaid config set search_merge_strategy union  # set union (default)
```

