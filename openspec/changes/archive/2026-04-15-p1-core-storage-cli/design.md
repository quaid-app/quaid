## Context

Phase 1 implements the first complete slice of GigaBrain: from empty database to a functional
knowledge brain that can be searched by keyword and semantics, accessed via MCP, and
round-tripped without data loss.

The repository scaffold (Sprint 0) exists: `Cargo.toml` with all declared dependencies,
module stubs with `todo!()` bodies, `src/schema.sql` (full v4 DDL), CI/CD workflows,
and `AGENTS.md`/`CLAUDE.md`. Every `.rs` file exists at the correct path; the job is
to replace stubs with working implementations in dependency order.

**Current state:**
- All source files exist as stubs (`todo!()` or empty)
- `src/schema.sql` is spec-complete (v4 DDL, embedded via `include_str!`)
- `Cargo.toml` declares all required dependencies (candle, rusqlite/bundled, sqlite-vec, rmcp, clap/env)
- CI: `cargo fmt` + `cargo clippy -- -D warnings` + `cargo check` + `cargo test` on every PR

**Constraints:**
- Single static binary (no dynamic SQLite, no ONNX runtime, no system dependencies)
- No network at runtime (model weights embedded via `include_bytes!` or optional `online-model` feature)
- Single writer (no multi-tenant, no replication, no auth)
- Rust edition 2021; MSRV not yet pinned — avoid `#[expect(...)]` (requires 1.81+)
- `thiserror` for `src/core/` error types; `anyhow` for CLI/commands

## Goals / Non-Goals

**Goals:**
- Replace all Phase 1 stubs with correct implementations
- Pass `cargo test` with round-trip and search correctness tests
- `gbrain import <corpus>` → `gbrain export` produces semantic-equivalent output
- `gbrain serve` connects to any MCP client exposing 5 core tools
- Static binary verifiable via `ldd` on Linux musl build
- OCC enforced on all write paths (version column, compare-and-swap)

**Non-Goals:**
- Graph traversal, assertions, contradiction detection (Phase 2)
- Progressive retrieval, palace room-level filtering (Phase 2)
- Knowledge gap tools in MCP (`brain_gap`, `brain_gaps`) (Phase 3)
- BEIR leaderboard benchmarks (Phase 3)
- `--json` on all commands, `pipe` mode, `call`, `version` polish (Phase 3)
- Room-level palace filtering; wing-only in Phase 1

## Decisions

### 1. SQLite connection management: one connection per command invocation

**Decision:** Open a single `rusqlite::Connection` per CLI invocation; pass it by mutable
reference through the call stack. No connection pool.

**Rationale:** GigaBrain is single-writer. CLI commands are short-lived processes. MCP
server is long-lived but still single-writer. A pool adds complexity for no benefit. WAL
mode handles concurrent readers at the OS level.

**Alternative considered:** `Arc<Mutex<Connection>>` for MCP server concurrency.
Rejected: rmcp processes tool calls sequentially by default; a mutex would serialize them
anyway. Revisit only if rmcp adds true parallel dispatch.

### 2. sqlite-vec loading: `unsafe` load from bundled bytes

**Decision:** Load sqlite-vec via `rusqlite::Connection::load_extension` from the bundled
static library at open time. Use `Connection::load_extension_enable()` guarded by
a feature flag check so CI without the extension can still run unit tests.

**Rationale:** sqlite-vec is a SQLite extension that must be loaded at runtime. The
`sqlite-vec` crate bundles the extension; loading it at open time keeps the rest of the
codebase unaware of the extension lifecycle.

**Alternative considered:** Compile sqlite-vec directly into rusqlite's bundled SQLite.
More complex build; deferred to a future hardening pass.

### 3. Candle embeddings: lazy singleton initialization

**Decision:** Initialize the candle model once per process in a `OnceLock<EmbeddingModel>`
in `src/core/inference.rs`. Commands that need embeddings call `ensure_model()` which
initializes on first call.

**Rationale:** Model init (tokenizer load + weight deserialization) takes ~500ms. We
don't want that cost on every command that happens not to use embeddings. Lazy init
keeps `gbrain get` and `gbrain search` fast.

**Alternative considered:** Separate `gbrain embed --daemon` process. Over-engineered
for v1; model init is a one-time cost per invocation.

### 4. Model weights: `include_bytes!` for offline default, `online-model` feature for smaller binary

**Decision:** Default build embeds BGE-small-en-v1.5 weights via `include_bytes!`
(~90MB binary). `--features online-model` skips embedding weights; binary downloads to
`~/.gbrain/models/` on first inference call.

**Rationale:** Spec requirement: zero network at runtime by default. The `online-model`
feature is for CI and developer builds where binary size matters.

### 5. Hybrid search: SMS short-circuit → FTS5 + vec0 fan-out → set-union merge

**Decision:** Implement in three stages inside `search.rs`:
1. **SMS (Shortest Match Scoring)**: if query is a quoted exact slug or wiki-link target,
   return direct page hit immediately (skip FTS5 + vec).
2. **FTS5 + vec0 fan-out**: run both in parallel (sequential is fine for v1, parallelism
   deferred), collect ranked result sets.
3. **Set-union merge**: combine result sets by slug deduplication, score by FTS5 BM25 +
   cosine similarity weighted sum. RRF available via `gbrain config set search_merge_strategy rrf`.

**Alternative considered:** Reciprocal Rank Fusion (RRF) as default. RRF normalizes rank
positions well but loses absolute score magnitude. Set-union preserves BM25 signal and is
simpler to reason about for a personal KB. Either can be selected at runtime.

### 6. MCP server: rmcp crate, stdio transport, sequential tool dispatch

**Decision:** Use `rmcp` 0.1 with the stdio transport. Register all 5 Phase 1 tools
(`brain_get`, `brain_put`, `brain_query`, `brain_search`, `brain_list`). Handle each
tool call by delegating to the same core functions used by the CLI.

**Rationale:** The spec mandates `gbrain serve` connects to Claude Code via MCP. `rmcp`
is already declared in `Cargo.toml` and implements the MCP stdio protocol. Reusing
core functions (not duplicating logic) means CLI and MCP always behave identically.

**Note:** MCP server needs a `tokio` runtime (already in `Cargo.toml` with `full` features).
Wrap the `main` of `serve` in `#[tokio::main]`.

### 7. OCC (Optimistic Concurrency Control) enforcement

**Decision:** All write paths (`put`, `import`, `ingest`, MCP `brain_put`) use a
compare-and-swap on the `version` column:
```sql
UPDATE pages SET ..., version = version + 1, updated_at = ...
WHERE slug = ? AND version = ?
```
If `rows_affected == 0`, return `ConflictError` with current version. CLI exits with
code 1. MCP returns JSON-RPC error code `-32009` with current version in data.

`brain_put` accepts optional `expected_version`. Omitted = treat as create (INSERT OR
IGNORE, then compare-and-swap with version=1 if row already exists is an error on
first-write paths; insert is unconditional for new pages).

### 8. Markdown round-trip: compiled_truth / timeline split at `---` boundary

**Decision:** `split_content(raw: &str) -> (String, String)` splits at the first
occurrence of a line containing only `---` (after frontmatter). Everything above is
`compiled_truth`; everything below is `timeline`. `render_page(page: &Page) -> String`
reconstructs: frontmatter YAML block + `\n` + compiled_truth + `\n---\n` + timeline.

**Rationale:** The spec defines `---` as the above-the-line / below-the-line boundary.
The split must be byte-exact to pass `roundtrip_raw.rs`.

### 9. Wing / room derivation: slug-prefix heuristic + section-header heuristic

**Decision:**
- `derive_wing(slug: &str) -> String`: take the first segment of the slug path
  (e.g. `people/alice-jones` → `people`). Fall back to `"general"` for flat slugs.
- `derive_room(content: &str) -> String`: scan `##` headers in compiled_truth;
  return the most-frequent heading as the room. Fall back to `""` for Phase 1
  (room-level filtering is deferred).

### 10. Error handling conventions

- `src/core/` modules use `thiserror`-derived error enums: `DbError`, `SearchError`,
  `InferenceError`, `ParseError`, `ConflictError`, `OccError`.
- `src/commands/` and `src/main.rs` use `anyhow::Result` with `?` propagation.
- `src/mcp/server.rs` maps core errors to JSON-RPC error codes (see Decision 7 for OCC).

## Risks / Trade-offs

**Risk: candle + tokenizers build time** → Mitigation: sccache or `cargo` incremental
builds in CI. The scaffold already has CI set up; add `CARGO_INCREMENTAL=1` if build
times exceed 10 minutes.

**Risk: sqlite-vec extension ABI mismatch with bundled rusqlite SQLite** → Mitigation:
ensure `sqlite-vec` crate version is compatible with `rusqlite = "0.31"` bundled SQLite.
Pin both versions and test on CI before merge.

**Risk: BGE-small-en-v1.5 embedding quality insufficient for personal KB recall** →
Mitigation: establish BEIR nDCG@10 baseline in Phase 1 ship gate. No regression gate
until Phase 3, but baseline must exist.

**Risk: rmcp 0.1 API instability** → Mitigation: pin exact rmcp version; if API breaks,
Fry owns the MCP server and can adapt. The 5 Phase 1 tools have stable schemas.

**Risk: OCC conflicts in MCP multi-agent sessions silently swallowing updates** →
Mitigation: Nibbler adversarial review on MCP server before Phase 1 ship gate (see
reviewer gates in proposal).

**Risk: Static binary size exceeds 90MB** → Mitigation: `opt-level = "z"` in release
profile; strip debug symbols; candle features whitelisted to text-only (no vision,
no audio). Track binary size in CI as informational metric.

## Migration Plan

Phase 1 is greenfield — no existing users, no existing database files. No migration
required. The `gbrain init` command creates a new `brain.db` from scratch.

For development iteration:
- Delete and re-init `brain.db` is the migration strategy during Phase 1 development.
- Schema versioning (`PRAGMA user_version`) is set to `4` in `schema.sql`; Phase 2
  will add migration logic if schema changes are needed.

## Open Questions

1. **candle device selection**: Should `gbrain` auto-detect CUDA/Metal and use GPU
   acceleration, or always use CPU? Decision: CPU only for Phase 1. GPU detection deferred
   to Phase 3 (adds `candle-core/cuda` + `candle-core/metal` feature flags and complex
   device selection logic).

2. **import concurrency**: Should `gbrain import` spawn multiple threads for embedding
   generation? Decision: single-threaded for Phase 1 (correctness first, performance later).
   Add `--jobs N` flag in Phase 3 if import is too slow on large corpora.

3. **MCP error code for non-OCC failures**: JSON-RPC error codes for `not_found`,
   `parse_error`, `db_error` are not yet defined. Use the standard JSON-RPC range
   (-32700 to -32603) for protocol errors and `-32001` to `-32099` for application-level
   errors. Define a full mapping table in `src/mcp/server.rs`.
