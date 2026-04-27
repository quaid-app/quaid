## ADDED Requirements

### Requirement: FTS5 full-text search
The system SHALL implement FTS5 keyword search over the `page_fts` virtual table using
BM25 scoring. Results SHALL be ranked by BM25 score (most relevant first). The `--wing`
flag SHALL restrict results to pages matching the given wing.

#### Scenario: Keyword search returns ranked results
- **WHEN** `search_fts("machine learning", None, &conn)` is called
- **THEN** pages whose `compiled_truth` or `title` contain those terms are returned in BM25-ranked order

#### Scenario: Wing-filtered FTS search
- **WHEN** `search_fts("fundraising", Some("companies"), &conn)` is called
- **THEN** only pages with `wing = 'companies'` are returned, still BM25-ranked

#### Scenario: FTS search on empty database
- **WHEN** `search_fts("anything", None, &conn)` is called on a fresh database
- **THEN** an empty result set is returned without error

### Requirement: quaid search command
`quaid search "<QUERY>"` SHALL invoke FTS5 search and print results as slug + summary
lines, ordered by relevance. `--wing <WING>` filters results. `--limit <N>` caps output
(default: 10).

#### Scenario: Search from CLI
- **WHEN** `quaid search "venture capital"` is called
- **THEN** up to 10 results are printed as `<slug>: <summary>` lines, most relevant first

#### Scenario: No results
- **WHEN** `quaid search "zzzznonexistent"` is called
- **THEN** "No results found." is printed and the command exits 0

### Requirement: SMS exact-match short-circuit
The hybrid search pipeline SHALL check for an exact slug match before invoking FTS5 or
vector search. If the query string exactly matches a page slug (or is wrapped in
`[[wiki-link]]` format), the matching page SHALL be returned immediately as the sole result.

#### Scenario: Exact slug match short-circuits search
- **WHEN** `hybrid_search("people/alice", &conn)` is called and a page with that exact slug exists
- **THEN** the page is returned immediately without FTS5 or vector fan-out

#### Scenario: Wiki-link format
- **WHEN** `hybrid_search("[[people/alice]]", &conn)` is called
- **THEN** the `[[` and `]]` are stripped and the slug match path is triggered

#### Scenario: No exact match falls through to full search
- **WHEN** `hybrid_search("who knows Jensen Huang", &conn)` is called
- **THEN** FTS5 and vector search are both executed and their results merged

### Requirement: Hybrid search with set-union merge
The `hybrid_search` function SHALL combine FTS5 and vector search results using
set-union merge: deduplicate by slug, score each result as a weighted combination of
BM25 rank and cosine similarity. The merge strategy SHALL be configurable via
`quaid config set search_merge_strategy rrf` to switch to Reciprocal Rank Fusion.

#### Scenario: Set-union deduplication
- **WHEN** FTS5 returns `[A, B, C]` and vector search returns `[B, C, D]`
- **THEN** hybrid_search returns `[A, B, C, D]` with merged scores, no duplicates

#### Scenario: Set-union merge default
- **WHEN** no config override is set
- **THEN** `hybrid_search` uses set-union merge strategy

#### Scenario: RRF merge via config
- **WHEN** `search_merge_strategy = "rrf"` is set in the config table
- **THEN** `hybrid_search` uses Reciprocal Rank Fusion to compute final scores

### Requirement: Wing-level palace filtering
The `hybrid_search` function SHALL accept an optional wing filter that restricts both
FTS5 and vector search fan-out to pages within the specified wing. Room-level filtering
is deferred to Phase 2.

#### Scenario: Wing filter restricts hybrid results
- **WHEN** `hybrid_search("investor meeting", &conn)` is called with wing filter `"people"`
- **THEN** both FTS5 and vec0 sub-queries are scoped to `wing = 'people'`
