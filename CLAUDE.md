# Quaid

Personal AI memory. SQLite + FTS5 + local vector embeddings. One binary.

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
memory.db                  — SQLite: pages + FTS5 + vec0 + links + assertions
```

**Thin harness, fat skills (mostly).** The binary is plumbing for *agent-facing* workflows: discovery, query, ingestion, and maintenance loops live in `skills/*/SKILL.md`. The exception is automated background intelligence — the conversation extraction prompt and the supersede/correction policy are hard-coded Rust in `src/core/conversation/` (e.g. `EXTRACTION_SYSTEM_PROMPT`, `supersede.rs`), not skills, because they run unattended in the daemon without an agent in the loop.

## Key files

| File                      | Purpose                                                                        |
| ------------------------- | ------------------------------------------------------------------------------ |
| `src/core/db.rs`          | rusqlite connection, schema init, WAL, sqlite-vec load                         |
| `src/core/types.rs`       | Page, Link, Tag, SearchResult, KnowledgeGap, etc.                              |
| `src/core/markdown.rs`    | `parse_frontmatter()`, `split_content()`, `extract_summary()`, `render_page()` |
| `src/core/fts.rs`         | FTS5 search: `search_fts(query, wing_filter, db)` → ranked results             |
| `src/core/inference.rs`   | candle init, `embed(text)`, `search_vec(query, k, wing_filter, db)`            |
| `src/core/search.rs`      | `hybrid_search(query, db)`: SMS + palace filter + FTS5 + vec + set-union       |
| `src/core/progressive.rs` | `progressive_retrieve(results, budget, depth)`: token-budget expansion         |
| `src/core/palace.rs`      | `derive_wing(slug)`, `derive_room(content)`, `classify_intent(query)`          |
| `src/core/novelty.rs`     | `check_novelty(content, page, db)`: Jaccard + cosine dedup                     |
| `src/core/assertions.rs`  | `check_assertions(slug, db)`: heuristic contradiction detection                |
| `src/core/graph.rs`       | `neighborhood_graph(slug, depth, db)`: N-hop BFS over links; emits `paths` |
| `src/core/gaps.rs`        | `log_gap()`, `list_gaps()`, `resolve_gap()`                                    |
| `src/core/chunking.rs`    | temporal sub-chunking: truth sections + individual timeline entries            |
| `src/core/links.rs`       | `extract_links()`, `resolve_slug()`, frontmatter edge/tag expansion, `upsert_derived_edge`, `sync_frontmatter_edges`, `sync_wikilink_edges` |
| `src/core/entities.rs`    | regex entity-pattern extraction (assertions only, no `links` writes); 5 ms per-page deadline, no inference/network |
| `src/core/migrate.rs`     | `export_dir()` plus round-trip export helpers                                  |
| `src/core/raw_imports.rs` | Active-source rotation, retention, and byte-exact restore support              |
| `src/core/collections.rs` | Collection metadata, slug resolution (`<collection>::<slug>`), lifecycle state |
| `src/core/reconciler.rs`  | Vault-tree vs DB diff/plan/apply: renames, quarantines, restore/remap safety pipeline |
| `src/core/vault_sync/`    | Live vault watcher + sync runtime: IPC, leases, write locks, restore/recovery flows |
| `src/core/conversation/`  | Conversation pipeline: turn capture, session close, SLM fact extraction, supersede/correction |
| `src/core/quarantine.rs`  | Quarantine workflow for pages whose vault file vanished or became unparseable  |
| `src/mcp/server.rs`       | MCP stdio server with all tools                                                |
| `src/schema.sql`          | Current DDL — embedded via `include_str!()`                                    |

## Build

```bash
# Debug
cargo build

# Release (single channel — provisions the configured model on first semantic use)
cargo build --release

# Offline-stub build (hash-shim embeddings only, no model download/provisioning)
cargo build --release --no-default-features --features bundled

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

Testing rules:
- Add new test coverage under organized files in `tests/...`; do not add new inline `#[cfg(test)] mod tests` blocks to production source files in `src/...`.
- Prefer subprocess or public-API integration tests for CLI/parser/runtime behavior. Do not add test-only production seams just to improve coverage unless the seam is part of the production design.

## Embedding model

Quaid defaults to `Qwen/Qwen3-Embedding-0.6B` (1024 dimensions, last-token
pooling, instruction-aware queries). Selection is via `QUAID_MODEL` or `--model`.

- `qwen3-0.6b` → `Qwen/Qwen3-Embedding-0.6B` (1024d, **default**; last-token pooling, `Instruct: …\nQuery: …` queries)
- `small` → `BAAI/bge-small-en-v1.5` (384d, CLS pooling)
- `base` (alias `medium`) → `BAAI/bge-base-en-v1.5` (768d)
- `large` → `BAAI/bge-large-en-v1.5` (1024d)
- `m3` (alias `max`) → `BAAI/bge-m3` (1024d, multilingual)
- any other value is treated as a full Hugging Face model ID

Aliases no longer carry pinned commit SHAs or file hashes (those rotted on HF
repo reorganisations); reproducibility rests on the `model_id` persisted in
`quaid_config`. Run `quaid model list` (`--json` for scripting) to see the
built-in aliases, their dimensions, and approximate download sizes. Downloads
use the model's `main` revision unless `--model-revision <sha>` pins one.

### Airgapped = local-only inference, not embedded weights

"Airgapped" means local-only inference (no cloud, no API keys, no data egress) —
**not** weights baked into the binary. There is a **single build/release channel**
(the former `embedded-model`/`online-model` split is gone): one binary per
platform that provisions the configured model on first semantic use (download +
verify + cache), then runs fully offline. The default models are too large to
`include_bytes!` (~1.2 GB embedder, ~2.5 GB GGUF extractor). A fully offline-stub
build is still possible via `--no-default-features --features bundled` (hash-shim
embeddings only). Tests force the deterministic hash shim with
`QUAID_FORCE_HASH_SHIM=1`.

Model metadata is persisted in the `quaid_config` table at `quaid init` and validated on every subsequent open. If the requested model differs from the initialized model, the command errors before touching embeddings (a pre-change 384d database must be re-initialized — no in-place 384→1024 migration is provided).

## Skills

Read `skills/` before doing brain operations. Agent-facing workflow intelligence lives there.
Skills are embedded in the binary by default. Override them by dropping a `SKILL.md` in
`~/.quaid/skills/<name>/` (user-global) or `./skills/<name>/` (working directory); the
embedded copy is never written to disk automatically. Materialize the embedded copies on
demand with `quaid skills extract` (`--force` overwrites local edits), then edit them in
place. Verify resolution and shadowing with `quaid skills doctor`.

## Database schema

See `src/schema.sql` for the current DDL. Key tables:
- `pages` — core content (compiled_truth + timeline markdown)
- `page_fts` — FTS5 virtual table (content-rowid, porter tokenizer)
- `quaid_config` — persisted `model_id`, `model_alias`, `embedding_dim`, `schema_version`
- `page_embeddings_vec_384` — vec0 virtual table for the default small model (additional vec tables are created for larger dimensions as needed)
- `page_embeddings` — chunk metadata + vec rowid join table
- `links` — typed temporal cross-references
- `assertions` — heuristic contradiction detection
- `knowledge_gaps` — queries the brain couldn't answer
- `raw_imports` — active source bytes plus bounded inactive history for byte-exact restore

## MCP tools

26 tools, registered in `src/mcp/server.rs` (`rmcp::tool_box!`):

- Pages: `memory_get`, `memory_put`, `memory_list`, `memory_raw`
- Search: `memory_query`, `memory_search`, `memory_rehydrate`
- Links: `memory_link`, `memory_link_close`, `memory_backlinks`, `memory_graph`
- Assertions: `memory_check`
- Tags/timeline: `memory_timeline`, `memory_tags`
- Gaps: `memory_gap`, `memory_gaps`, `memory_gap_resolve`
- Conversation: `memory_add_turn`, `memory_close_session`, `memory_close_action`,
  `memory_correct`, `memory_correct_continue`
- Admin: `memory_stats`, `memory_collections`, `memory_namespace_create`, `memory_namespace_destroy`

## Optimistic concurrency

`memory_put` accepts an optional `expected_version`. If the page's current version
doesn't match, the call returns `ConflictError`. Always re-fetch before writing.
