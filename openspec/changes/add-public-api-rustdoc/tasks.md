## 1. Pre-flight

- [x] 1.1 Confirm `decompose-vault-sync-module` is archived under `openspec/changes/archive/`
- [x] 1.2 Confirm `decompose-mcp-server-module` is archived under `openspec/changes/archive/`
- [x] 1.3 Re-inventory `src/core/` and `src/mcp/` (`ls` and `grep -c '^pub ' src/core/**/*.rs src/mcp/**/*.rs`) and update tasks below if module counts changed materially since this proposal was authored

## 2. Crate root

- [x] 2.1 Add multi-paragraph `//!` doc to `src/lib.rs`: elevator paragraph, module-map paragraph (`core` / `mcp` / `commands`), "where to start" paragraph (consumers → `mcp::server`, `core::conversation`; maintainers → `core::db`, `core::vault_sync`)
- [x] 2.2 Add `#![warn(missing_docs)]` to `src/lib.rs` at the top, below the crate `//!` block
- [x] 2.3 Run `cargo doc --no-deps 2>&1 | tee /tmp/quaid-doc-baseline.log` and confirm warnings fire on every undoc'd `pub` item — this is the workload baseline

## 3. `src/core/` — primitives and storage

- [x] 3.1 `core/mod.rs`: add `//!` module doc plus `///` for any re-exports that narrow the public façade
- [x] 3.2 `core/db.rs`: `//!` header + `///` for every `pub` item (rusqlite connection, schema init, WAL, sqlite-vec load)
- [x] 3.3 `core/types.rs`: `//!` header + `///` for `Page`, `Link`, `Tag`, `SearchResult`, `KnowledgeGap`, and every other public type/variant/field
- [x] 3.4 `core/markdown.rs`: `//!` header + `///` for `parse_frontmatter`, `split_content`, `extract_summary`, `render_page`
- [x] 3.5 `core/page_uuid.rs`, `core/namespace.rs`, `core/file_state.rs`, `core/fs_safety.rs`, `core/ignore_patterns.rs`: `//!` headers + `///` on all `pub` items
- [x] 3.6 Run `cargo doc --no-deps 2>&1 | grep -c warning` and verify the count drops monotonically as each section is committed

## 4. `src/core/` — search, retrieval, palace

- [x] 4.1 `core/fts.rs`: confirm existing docs match the bar; `//!` header; doc any uncovered `pub` items
- [x] 4.2 `core/inference.rs`: `//!` header + `///` for candle init, `embed`, `search_vec`, and every other public item; document feature-gated items so `online-model` build is also clean
- [x] 4.3 `core/search.rs`: `//!` header + `///` for `hybrid_search` and helpers
- [x] 4.4 `core/progressive.rs`, `core/palace.rs`, `core/novelty.rs`, `core/chunking.rs`: `//!` headers + `///` on all `pub` items
- [x] 4.5 `core/links.rs`, `core/graph.rs`: `//!` headers + `///` on `extract_links`, `resolve_slug`, temporal-validity helpers, `neighborhood_graph`

## 5. `src/core/` — semantics, gaps, migrate, raw

- [x] 5.1 `core/assertions.rs`: `//!` + `///` for `check_assertions` and contradiction-detection helpers
- [x] 5.2 `core/gaps.rs`: `//!` + `///` for `log_gap`, `list_gaps`, `resolve_gap`
- [x] 5.3 `core/migrate.rs`: `//!` + `///` for `export_dir` and round-trip helpers
- [x] 5.4 `core/raw_imports.rs`: `//!` + `///` covering active-source rotation, retention, byte-exact restore
- [x] 5.5 `core/collections.rs`, `core/quarantine.rs`, `core/reconciler.rs`, `core/supersede.rs`: `//!` + `///` on all `pub` items

## 6. `src/core/conversation/` (extraction pipeline)

- [x] 6.1 `conversation/mod.rs`: `//!` describing the pipeline (turn capture → queue → extractor → writer → janitor) and a "see also" tour of the submodules
- [x] 6.2 `conversation/queue.rs`: confirm `enqueue` / `enqueue_force_path` docs (already added in `fix-extraction-force-correctness`) match the bar; `//!` header; doc any remaining `pub` items
- [x] 6.3 `conversation/extractor.rs`, `conversation/turn_writer.rs`, `conversation/idle_close.rs`, `conversation/janitor.rs`: `//!` + `///` on all `pub` items
- [x] 6.4 `conversation/correction.rs`, `conversation/file_edit.rs`, `conversation/format.rs`: `//!` + `///` on all `pub` items
- [x] 6.5 `conversation/slm.rs`, `conversation/model_lifecycle.rs`, `conversation/supersede.rs`: `//!` + `///` on all `pub` items

## 7. `src/core/vault_sync/` (post-#4 split)

- [x] 7.1 `core/vault_sync/mod.rs`: `//!` describing the vault-sync façade and a "see also" map of every submodule introduced by `decompose-vault-sync-module`
- [x] 7.2 For each submodule under `core/vault_sync/`: add `//!` header + `///` on every `pub` item. (Specific list intentionally deferred to the post-#4 inventory; cross-check against task 1.3.)
- [x] 7.3 Verify no `pub` item from the original `vault_sync.rs` lost its doc during the split

## 8. `src/mcp/` (post-#5 split)

- [x] 8.1 `mcp/mod.rs`: `//!` describing the MCP layer (stdio JSON-RPC server + tool surface) and a "see also" map of submodules
- [x] 8.2 `mcp/server.rs` (or its post-split equivalent): `//!` + `///` on every `pub` item, including each MCP tool handler
- [x] 8.3 For each additional submodule introduced by `decompose-mcp-server-module`: `//!` + `///` on every `pub` item
- [x] 8.4 Confirm tool-name docs match the public tool list in `CLAUDE.md` ("MCP tools" section) so consumers reading rustdoc can trust the surface

## 9. CI gate

- [x] 9.1 Locate the CI lint job (post `add-rust-lints-and-ci-gate` if landed; otherwise the existing build job)
- [x] 9.2 Add a step: `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`
- [x] 9.3 Add a sibling step: `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --no-default-features --features bundled,online-model`
- [x] 9.4 Verify both steps pass on a fresh CI run on this branch

## 10. Verification and polish

- [x] 10.1 Local clean-build check (default features): `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features` exits 0 with no warnings
- [x] 10.2 Local clean-build check (online-model channel): `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --no-default-features --features bundled,online-model` exits 0 with no warnings
- [x] 10.3 Open `target/doc/quaid/index.html` and visually confirm: crate intro renders, every module under `core` and `mcp` shows a description on the index, "where to start" links click through correctly
- [x] 10.4 Spot-check 5 random `pub` items per module group: doc is one full sentence of intent, not a signature restatement
- [x] 10.5 Run `cargo test` to ensure no doc-comment Markdown is interpreted as a doctest that now fails (doctests can be silently introduced by triple-backtick fences inside `///`)
- [x] 10.6 Update `docs/CODE_REVIEW.md` §6.1 with a one-line note that the gap has been closed and reference this change
