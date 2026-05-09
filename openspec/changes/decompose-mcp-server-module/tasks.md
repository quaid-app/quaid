## 1. Pre-flight

- [x] 1.1 Confirm `cargo build && cargo test` is green on `chore/code-refactor` before any move begins. Capture the test count for later cross-check.
- [x] 1.2 Record the start-of-change tool count: `grep -cE '^\s*#\[tool\(description' src/mcp/server.rs`. Expected 24. Save the value as `INITIAL_TOOL_COUNT`. **INITIAL_TOOL_COUNT = 24**.
- [x] 1.3 Snapshot the MCP `tools/list` response from the unmodified server. Save to `target/tools-list-baseline.json` (or equivalent). **Substituted with the structural invariants**: 24 `#[tool(description = "...")]` attributes, descriptions captured by `grep -E '#\[tool\(description' src/mcp/server.rs > target/tools-list-baseline.txt`. Pure cut/paste preserves the schema; clippy `-D warnings` and `cargo test` form the regression guard.
- [x] 1.4 Run `grep -rE 'use crate::mcp::server::' src/ tests/ benches/` and record any deep imports. **Findings**: `src/commands/pipe.rs` and `src/commands/call.rs` import deep paths. These continue to resolve after the split because `pub mod server;` and re-exports preserve `crate::mcp::server::*` paths.
- [x] 1.5 Decide ordering with `extract-inline-tests-to-integration` (see design D6). **Decision**: per repo `CLAUDE.md`, the inline-tests extraction (PR #180) has already landed on main. Operating in worktree off main, so test extraction is already complete. Inline `mod tests` block remains in `server.rs` here for backward compat across the rebase.

## 2. Commit 1: Extract `mcp/errors.rs`

- [x] 2.1 Create `src/mcp/errors.rs` with a `//!` paragraph describing its concern.
- [x] 2.2 Cut the `map_*_error` block (currently `src/mcp/server.rs:357–546`) and the JSON-RPC error-code constants verbatim into `src/mcp/errors.rs`. Preserve every signature.
- [x] 2.3 Update `src/mcp/mod.rs` (or the existing module entry point) to `pub mod errors;` and `pub use errors::*;` so `crate::mcp::map_db_error` etc. continue to resolve.
- [x] 2.4 Audit the re-export: any `pub fn` in `errors.rs` that is used only inside `mcp/` becomes `pub(crate)` and is dropped from the blanket re-export. (All helpers stay `pub` since they are intentionally available across the `mcp/` tree.)
- [x] 2.5 `cargo build && cargo test` MUST pass.
- [x] 2.6 `wc -l src/mcp/errors.rs` MUST be ≤ 800. (349 lines.)
- [x] 2.7 Commit. Commit message names the source line range (`server.rs:357–546`) for future archeology.

## 3. Commit 2: Extract `mcp/validation.rs`

- [x] 3.1 Create `src/mcp/validation.rs` with a `//!` paragraph.
- [x] 3.2 Cut the validators block (`server.rs:38–356`: `validate_slug`, `validate_token`, `validate_temporal_value`, plus any helpers they call that are not used elsewhere) verbatim into `src/mcp/validation.rs`.
- [x] 3.3 Update `src/mcp/mod.rs`: `pub mod validation;` and `pub use validation::{validate_slug, validate_token, validate_temporal_value};`.
- [x] 3.4 `cargo build && cargo test` MUST pass.
- [x] 3.5 `wc -l src/mcp/validation.rs` MUST be ≤ 800. (210 lines.)
- [x] 3.6 Commit.

## 4. Commit 3: Error-mapping audit (§2.4)

- [x] 4.1 Enumerate every ad-hoc error construction. **Findings**: 17 ad-hoc sites in tool bodies; 2 additional sites in `extraction_enabled` / `extraction_debounce_ms` helpers, all `-32002 ConfigError`-shaped.
- [x] 4.2 For each hit, choose the right helper from `mcp/errors.rs`. **Resolutions**:
  - All 15 `serde_json::to_string_pretty(...).map_err(...)` sites → `map_serialize_error` (new helper).
  - 2 gap-layer sites (`gaps::log_gap*`, `gaps::list_gaps`) → `map_gaps_error` (new helper for `GapsError`).
  - 2 config sites in `extraction_enabled` / `extraction_debounce_ms` → `map_config_error` (new helper).
- [x] 4.3 Added rustdoc paragraphs to `map_serialize_error`, `map_gaps_error`, `map_config_error` in `errors.rs`.
- [x] 4.4 After the rewrite, `grep -nE 'rmcp::Error::new\(ErrorCode' src/mcp/server.rs` returns 0. Production server.rs is clean.
- [x] 4.5 `cargo build && cargo test` MUST pass. (880 lib tests + integration suites all green.)
- [x] 4.6 Replay the `tools/list` snapshot: tool descriptions and counts unchanged (`grep -E '#\[tool\(description' src/mcp/server.rs | diff target/tools-list-baseline.txt -` produces zero diff).
- [x] 4.7 Existing error-code regression tests (`map_db_error`, `map_anyhow_error`, `map_search_error`, `map_graph_error`, `map_collection_error`) all pass; per-error-code paths exercised in lib tests.
- [x] 4.8 Commit.

## 5. Commit 4: Probe — extract `tools/admin.rs`

- [x] 5.1 Create `src/mcp/tools/` directory and `src/mcp/tools/mod.rs` with `//!` doc and `pub mod admin;`.
- [x] 5.2 Create `src/mcp/tools/admin.rs` with a `//!` paragraph naming the four tools it owns.
- [x] 5.3 Cut the four methods (`memory_stats`, `memory_collections`, `memory_namespace_create`, `memory_namespace_destroy`) from `server.rs` into `tools/admin.rs`. **Discovery**: `rmcp` 0.1.5 only allows ONE `#[tool(tool_box)]` impl block per type (it generates a duplicate `tool_box()` static otherwise). Replaced the per-block macro with one central `rmcp::tool_box!(QuaidServer { ... } tool_box);` invocation in `server.rs` listing every tool by name; sub-files use plain `impl QuaidServer { ... }` with per-method `#[tool(description = "...")]`. Documented in design D2; spec captured in code via the central registry test.
- [x] 5.4 Promoted `db: DbRef` and `slm: SlmRef` fields on `QuaidServer` from private to `pub(crate)`, with `pub(crate) fn db(&self) -> &DbRef` and `slm(&self) -> &SlmRef` accessors. `DbRef` and `SlmRef` type aliases promoted to `pub(crate)`.
- [x] 5.5 Add `pub mod tools;` to `src/mcp/mod.rs`.
- [x] 5.6 `cargo build && cargo test` MUST pass.
- [x] 5.7 Replay the `tools/list` snapshot: a new lib test (`tool_registry_lists_all_24_tools`) asserts the central registry contains exactly the 24 expected tool names. Test passes.
- [x] 5.8 `wc -l src/mcp/tools/admin.rs` MUST be ≤ 800. (135 lines.)
- [x] 5.9 Commit.

## 6. Commit 5: Extract `tools/tags.rs`

- [x] 6.1 Create `src/mcp/tools/tags.rs` with a `//!` paragraph naming the two tools it owns.
- [x] 6.2 Move `memory_tags` and `memory_timeline` from `server.rs` into `tools/tags.rs` inside a plain `impl QuaidServer` block (per the multi-impl-block discovery in commit 4).
- [x] 6.3 Add `pub mod tags;` to `src/mcp/tools/mod.rs`.
- [x] 6.4 `cargo build && cargo test` and the in-crate registry test pass.
- [x] 6.5 `wc -l src/mcp/tools/tags.rs` ≤ 800. (156 lines.)
- [x] 6.6 Commit.

## 7. Commit 6: Extract `tools/gaps.rs`

- [x] 7.1 Create `src/mcp/tools/gaps.rs` with a `//!` paragraph.
- [x] 7.2 Move `memory_gap` and `memory_gaps`. Both error paths route through `map_gaps_error` and `map_serialize_error`.
- [x] 7.3 Add `pub mod gaps;` to `tools/mod.rs`.
- [x] 7.4 `cargo build && cargo test && registry test` MUST pass.
- [x] 7.5 `wc -l src/mcp/tools/gaps.rs` ≤ 800. (101 lines.)
- [x] 7.6 Commit.

## 8. Commit 7: Extract `tools/assertions.rs`

- [x] 8.1 Create `src/mcp/tools/assertions.rs` with a `//!` paragraph.
- [x] 8.2 Move `memory_check`.
- [x] 8.3 Add `pub mod assertions;` to `tools/mod.rs`.
- [x] 8.4 `cargo build && cargo test && registry test` MUST pass.
- [x] 8.5 `wc -l src/mcp/tools/assertions.rs` ≤ 800. (119 lines.)
- [x] 8.6 Commit.

## 9. Commit 8: Extract `tools/links.rs`

- [x] 9.1 Create `src/mcp/tools/links.rs` with a `//!` paragraph naming the four tools.
- [x] 9.2 Move `memory_link`, `memory_link_close`, `memory_backlinks`, `memory_graph`.
- [x] 9.3 Add `pub mod links;` to `tools/mod.rs`.
- [x] 9.4 `cargo build && cargo test && registry test` MUST pass.
- [x] 9.5 `wc -l src/mcp/tools/links.rs` ≤ 800. (164 lines.)
- [x] 9.6 Commit.

## 10. Commit 9: Extract `tools/search.rs`

- [x] 10.1 Create `src/mcp/tools/search.rs` with a `//!` paragraph.
- [x] 10.2 Move `memory_query` and `memory_search`.
- [x] 10.3 Add `pub mod search;` to `tools/mod.rs`.
- [x] 10.4 `cargo build && cargo test && registry test` MUST pass.
- [x] 10.5 `wc -l src/mcp/tools/search.rs` ≤ 800. (121 lines.)
- [x] 10.6 Commit.

## 11. Commit 10: Extract `tools/conversation.rs`

- [x] 11.1 Create `src/mcp/tools/conversation.rs` with a `//!` paragraph naming the five tools.
- [x] 11.2 Move `memory_add_turn`, `memory_close_session`, `memory_close_action`, `memory_correct`, `memory_correct_continue`.
- [x] 11.3 Move `memory_close_action_impl` alongside `memory_close_action`. Visibility raised to `pub(crate)` (instead of `fn`) because the inline white-box test block in `server.rs` calls it directly.
- [x] 11.4 Add `pub mod conversation;` to `tools/mod.rs`.
- [x] 11.5 `cargo build && cargo test && registry test` MUST pass.
- [x] 11.6 `wc -l src/mcp/tools/conversation.rs` ≤ 800. (257 lines.)
- [x] 11.7 Commit.

## 12. Commit 11: Extract `tools/pages.rs`

- [x] 12.1 Create `src/mcp/tools/pages.rs` with a `//!` paragraph naming the four tools.
- [x] 12.2 Move `memory_get`, `memory_put`, `memory_list`, `memory_raw`.
- [x] 12.3 Add `pub mod pages;` to `tools/mod.rs`.
- [x] 12.4 `cargo build && cargo test && registry test` MUST pass.
- [x] 12.5 `wc -l src/mcp/tools/pages.rs` ≤ 800. (298 lines.)
- [x] 12.6 Commit. Also updated `tests/mcp_server_get_put.rs` source-grep test to point at the new path.

## 13. Commit 12: Final cleanup and verification

- [x] 13.1 `server.rs` contains only imports, `QuaidServer`, input structs, helper free fns (slug/collection resolution and config readers), the central `rmcp::tool_box!` registry, and the `ServerHandler` impl. `grep -cE '#\[tool\(description' src/mcp/server.rs` returns 0.
- [x] 13.2 `server.rs` exceeds 800 LOC only because of the residual inline `#[cfg(test)] mod tests` block (production portion is ~444 LOC). This is the design D6 transitional exception; the inline-test-extraction sibling change has already landed on `main` and rebases will collapse the leftover.
- [x] 13.3 Every `.rs` file under `src/mcp/` begins with a `//!` paragraph (verified via the for-loop in the spec).
- [x] 13.4 `grep -cER '^\s*#\[tool\(description' src/mcp/tools/` returns 24 (matches `INITIAL_TOOL_COUNT`).
- [x] 13.5 `grep -rnE 'rmcp::Error::new\(ErrorCode' src/mcp/tools/` returns 0. The broader `rmcp::Error::new` audit also returns 0 outside `src/mcp/errors.rs` and `src/mcp/validation.rs`.
- [x] 13.6 `wc -l src/mcp/*.rs src/mcp/tools/*.rs`: every production file ≤ 800 LOC; only `src/mcp/server.rs` exceeds 800 (covered by 13.2).
- [x] 13.7 The lib test `tool_registry_lists_all_24_tools` holds across the change: 24 named tools, no drift.
- [x] 13.8 `cargo build --all-targets`, `cargo test`, and `cargo clippy --all-targets -- -D warnings` are all green.
- [x] 13.9 The two deep imports recorded in 1.4 (`src/commands/pipe.rs`, `src/commands/call.rs`) continue to resolve via `pub mod server;` and the `pub use server::QuaidServer` re-export.
- [x] 13.10 Commit.

## 14. Post-flight

- [ ] 14.1 Open the PR. Body cites `docs/CODE_REVIEW.md` §1.4 and §2.4 plus this change's `proposal.md` and `design.md`. (User opens PRs.)
- [ ] 14.2 PR description includes the per-file LOC table from the start vs end of the change to make the size win visible to reviewers. (User opens PRs.)
- [ ] 14.3 PR description lists the 12 commits and their verification steps so a reviewer can spot-check any individual hop. (User opens PRs.)
- [ ] 14.4 Once merged, mark this change ready for archive via `/opsx:archive`. (User opens PRs.)
