# GigaBrain

Personal knowledge brain. SQLite + FTS5 + local vector embeddings. One binary.

## Architecture

```
Consumers (Claude Code, any MCP client)
    ↓ stdio JSON-RPC 2.0
src/mcp/server.rs          — MCP tool definitions + handlers
    ↓
src/main.rs                — clap CLI dispatch
    ↓
src/commands/              — one file per command
    ↓
src/core/                  — library: DB, search, embeddings, parsing
    ↓
brain.db                   — SQLite: pages + FTS5 + vec0 + links + assertions
```

**Thin harness, fat skills.** The binary is plumbing. All agent workflows live in `skills/*/SKILL.md`.

## Key files

| File | Purpose |
|------|---------|
| `src/core/db.rs` | rusqlite connection, schema init, WAL, sqlite-vec load |
| `src/core/types.rs` | Page, Link, Tag, SearchResult, KnowledgeGap, etc. |
| `src/core/markdown.rs` | `parse_frontmatter()`, `split_content()`, `extract_summary()`, `render_page()` |
| `src/core/fts.rs` | FTS5 search: `search_fts(query, wing_filter, db)` → ranked results |
| `src/core/inference.rs` | candle init, `embed(text)`, `search_vec(query, k, wing_filter, db)` |
| `src/core/search.rs` | `hybrid_search(query, db)`: SMS + palace filter + FTS5 + vec + set-union |
| `src/core/progressive.rs` | `progressive_retrieve(results, budget, depth)`: token-budget expansion |
| `src/core/palace.rs` | `derive_wing(slug)`, `derive_room(content)`, `classify_intent(query)` |
| `src/core/novelty.rs` | `check_novelty(content, page, db)`: Jaccard + cosine dedup |
| `src/core/assertions.rs` | `check_assertions(slug, db)`: heuristic contradiction detection |
| `src/core/graph.rs` | `neighborhood_graph(slug, depth, db)`: N-hop BFS over links |
| `src/core/gaps.rs` | `log_gap()`, `list_gaps()`, `resolve_gap()` |
| `src/core/chunking.rs` | temporal sub-chunking: truth sections + individual timeline entries |
| `src/core/links.rs` | `extract_links()`, `resolve_slug()`, temporal validity |
| `src/core/migrate.rs` | `import_dir()`, `export_dir()`, `validate_roundtrip()` |
| `src/mcp/server.rs` | MCP stdio server with all tools |
| `src/schema.sql` | v4 DDL — embedded via `include_str!()` |

## Build

```bash
# Debug
cargo build

# Release (~90MB with embedded BGE-small-en-v1.5 model weights)
cargo build --release

# Cross-compile
cargo install cross
cross build --release --target aarch64-apple-darwin
cross build --release --target x86_64-apple-darwin
cross build --release --target x86_64-unknown-linux-musl
cross build --release --target aarch64-unknown-linux-musl
```

## Test

```bash
cargo test
# Key: tests/roundtrip_semantic.rs (normalized export) + tests/roundtrip_raw.rs (byte-exact)
```

## Embedding model

BGE-small-en-v1.5 via candle (pure Rust). 384 dimensions. Model weights embedded
via `include_bytes!()`. Zero dependencies, zero network required at runtime.
Build with `--features online-model` for a smaller binary that downloads on first run.

## Skills

Read `skills/` before doing brain operations. All workflow intelligence lives there.
Skills are embedded in the binary and extracted to `~/.gbrain/skills/` on first run.
Drop a custom `SKILL.md` in your working directory to override any default.

## Database schema

See `src/schema.sql` for the full v4 DDL. Key tables:
- `pages` — core content (compiled_truth + timeline markdown)
- `page_fts` — FTS5 virtual table (content-rowid, porter tokenizer)
- `page_embeddings_vec_384` — vec0 virtual table for BGE-small (created at init)
- `page_embeddings` — chunk metadata + vec rowid join table
- `links` — typed temporal cross-references
- `assertions` — heuristic contradiction detection
- `knowledge_gaps` — queries the brain couldn't answer
- `ingest_log` — SHA-256 idempotency audit trail

## MCP tools

Core (Phase 1): `brain_get`, `brain_put`, `brain_query`, `brain_search`, `brain_list`

Full surface (Phase 2+): `brain_link`, `brain_link_close`, `brain_backlinks`, `brain_graph`,
`brain_timeline`, `brain_tags`, `brain_check`, `brain_gap`, `brain_gaps`, `brain_stats`, `brain_raw`

## Optimistic concurrency

`brain_put` accepts an optional `expected_version`. If the page's current version
doesn't match, the call returns `ConflictError`. Always re-fetch before writing.
