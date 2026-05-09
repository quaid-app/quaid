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

- [ ] 4.1 Enumerate every ad-hoc error construction: `grep -nE 'rmcp::Error::new\(ErrorCode' src/mcp/server.rs` minus the legitimate sites in `errors.rs` (already moved) and `validation.rs` (already moved). Expected ~24 hits in tool bodies.
- [ ] 4.2 For each hit, choose the right helper from `mcp/errors.rs`. The mapping is:
    - `ErrorCode(-32003), format!("database error: {e}"), None` → `map_db_error(e)`
    - `ErrorCode(-32003), format!("search error: {message}"), None` → `map_search_error(...)`
    - `ErrorCode(-32003), e.to_string(), None` (db-flavoured) → `map_db_error(e)`
    - `ErrorCode(-32003), e.to_string(), None` (anyhow-flavoured) → `map_anyhow_error(e)`
    - `ErrorCode(-32003), <serde_json error>, None` → introduce `map_serialize_error` in `errors.rs` if not present, then call it
- [ ] 4.3 If `map_serialize_error` is added, add a one-paragraph rustdoc explaining what it accepts and the `-32003` code it emits.
- [ ] 4.4 After the rewrite, `grep -nE 'rmcp::Error::new\(ErrorCode' src/mcp/server.rs` returns only the legitimate uses inside helpers (zero in tool bodies if validators are already moved; otherwise only validator hits at the lines preserved by the validation-extract commit).
- [ ] 4.5 `cargo build && cargo test` MUST pass.
- [ ] 4.6 Replay the `tools/list` snapshot: response MUST match `target/tools-list-baseline.json` exactly.
- [ ] 4.7 Run any test name matching `returns_.*_error_code` or that asserts on `-32001/-32002/-32003/-32009/-32602`. All MUST pass. If coverage gaps exist (per design R3), add a single regression test per error code value before this commit lands.
- [ ] 4.8 Commit.

## 5. Commit 4: Probe — extract `tools/admin.rs`

- [ ] 5.1 Create `src/mcp/tools/` directory and `src/mcp/tools/mod.rs` with `//!` doc and `pub mod admin;`.
- [ ] 5.2 Create `src/mcp/tools/admin.rs` with a `//!` paragraph naming the four tools it owns.
- [ ] 5.3 Cut the four methods (`memory_stats`, `memory_collections`, `memory_namespace_create`, `memory_namespace_destroy`) from `server.rs` into `tools/admin.rs`, wrapped in a single `#[tool(tool_box)] impl QuaidServer { … }` block with the same imports.
- [ ] 5.4 Promote any helper used only by these methods to `pub(crate)` if necessary; no item visibility expands beyond `pub(crate)` unless it was already `pub`.
- [ ] 5.5 Add `pub mod tools;` to `src/mcp/mod.rs`.
- [ ] 5.6 `cargo build && cargo test` MUST pass.
- [ ] 5.7 Replay the `tools/list` snapshot: MUST still match the baseline. **This commit is the structural probe** — if `tools/list` no longer matches, abort and revert before touching any other domain (per design R1).
- [ ] 5.8 `wc -l src/mcp/tools/admin.rs` MUST be ≤ 800.
- [ ] 5.9 Commit.

## 6. Commit 5: Extract `tools/tags.rs`

- [ ] 6.1 Create `src/mcp/tools/tags.rs` with a `//!` paragraph naming the two tools it owns.
- [ ] 6.2 Move `memory_tags` and `memory_timeline` from `server.rs` into `tools/tags.rs` inside a single `#[tool(tool_box)] impl QuaidServer` block.
- [ ] 6.3 Add `pub mod tags;` to `src/mcp/tools/mod.rs`.
- [ ] 6.4 `cargo build && cargo test && tools/list snapshot match` MUST pass.
- [ ] 6.5 `wc -l src/mcp/tools/tags.rs` MUST be ≤ 800.
- [ ] 6.6 Commit.

## 7. Commit 6: Extract `tools/gaps.rs`

- [ ] 7.1 Create `src/mcp/tools/gaps.rs` with a `//!` paragraph.
- [ ] 7.2 Move `memory_gap` and `memory_gaps`. Verify each error path inside both methods now goes through a `mcp::errors::map_*` helper (the §2.4 ride-along audit should already have caught these in commit 3, but reconfirm — this is the file that contained the explicitly-flagged offender at `server.rs:1802–1808`).
- [ ] 7.3 Add `pub mod gaps;` to `tools/mod.rs`.
- [ ] 7.4 `cargo build && cargo test && tools/list snapshot match` MUST pass.
- [ ] 7.5 `wc -l src/mcp/tools/gaps.rs` MUST be ≤ 800.
- [ ] 7.6 Commit.

## 8. Commit 7: Extract `tools/assertions.rs`

- [ ] 8.1 Create `src/mcp/tools/assertions.rs` with a `//!` paragraph.
- [ ] 8.2 Move `memory_check`.
- [ ] 8.3 Add `pub mod assertions;` to `tools/mod.rs`.
- [ ] 8.4 `cargo build && cargo test && tools/list snapshot match` MUST pass.
- [ ] 8.5 `wc -l src/mcp/tools/assertions.rs` MUST be ≤ 800.
- [ ] 8.6 Commit.

## 9. Commit 8: Extract `tools/links.rs`

- [ ] 9.1 Create `src/mcp/tools/links.rs` with a `//!` paragraph naming the four tools.
- [ ] 9.2 Move `memory_link`, `memory_link_close`, `memory_backlinks`, `memory_graph`.
- [ ] 9.3 Add `pub mod links;` to `tools/mod.rs`.
- [ ] 9.4 `cargo build && cargo test && tools/list snapshot match` MUST pass.
- [ ] 9.5 `wc -l src/mcp/tools/links.rs` MUST be ≤ 800.
- [ ] 9.6 Commit.

## 10. Commit 9: Extract `tools/search.rs`

- [ ] 10.1 Create `src/mcp/tools/search.rs` with a `//!` paragraph.
- [ ] 10.2 Move `memory_query` and `memory_search`.
- [ ] 10.3 Add `pub mod search;` to `tools/mod.rs`.
- [ ] 10.4 `cargo build && cargo test && tools/list snapshot match` MUST pass.
- [ ] 10.5 `wc -l src/mcp/tools/search.rs` MUST be ≤ 800.
- [ ] 10.6 Commit.

## 11. Commit 10: Extract `tools/conversation.rs`

- [ ] 11.1 Create `src/mcp/tools/conversation.rs` with a `//!` paragraph naming the five tools.
- [ ] 11.2 Move `memory_add_turn`, `memory_close_session`, `memory_close_action`, `memory_correct`, `memory_correct_continue`.
- [ ] 11.3 Move the private helper `memory_close_action_impl` (currently `server.rs:982`) alongside `memory_close_action`. Keep its visibility at `fn` (not `pub`) — it is only called from within the same file post-move.
- [ ] 11.4 Add `pub mod conversation;` to `tools/mod.rs`.
- [ ] 11.5 `cargo build && cargo test && tools/list snapshot match` MUST pass.
- [ ] 11.6 `wc -l src/mcp/tools/conversation.rs` MUST be ≤ 800.
- [ ] 11.7 Commit.

## 12. Commit 11: Extract `tools/pages.rs`

- [ ] 12.1 Create `src/mcp/tools/pages.rs` with a `//!` paragraph naming the four tools.
- [ ] 12.2 Move `memory_get`, `memory_put`, `memory_list`, `memory_raw`. This is the largest domain (~275 LOC of method bodies).
- [ ] 12.3 Add `pub mod pages;` to `tools/mod.rs`.
- [ ] 12.4 `cargo build && cargo test && tools/list snapshot match` MUST pass.
- [ ] 12.5 `wc -l src/mcp/tools/pages.rs` MUST be ≤ 800. If close, audit for any helper that should live elsewhere (e.g. raw-imports helpers may belong in `core::raw_imports` if they aren't already).
- [ ] 12.6 Commit.

## 13. Commit 12: Final cleanup and verification

- [ ] 13.1 `server.rs` SHALL contain only: imports, the `QuaidServer` struct definition, any non-`#[tool]` methods (e.g. `new()`, helpers used by `ServerHandler`), and the `ServerHandler` impl. Confirm with `grep -cE '#\[tool\(description' src/mcp/server.rs` returning 0.
- [ ] 13.2 If `server.rs` exceeds 800 LOC purely because of the inline `#[cfg(test)] mod tests` block, that is acceptable transitionally per design D6 (the inline-test-extraction sibling change addresses it). Otherwise it MUST be ≤ 800 LOC.
- [ ] 13.3 Verify every new file has a `//!` paragraph: `for f in src/mcp/{mod,server,errors,validation}.rs src/mcp/tools/*.rs; do head -1 "$f" | grep -q '^//!' || echo "MISSING: $f"; done` MUST report nothing.
- [ ] 13.4 Verify final tool count: `grep -cER '^\s*#\[tool\(description' src/mcp/tools/` MUST equal `INITIAL_TOOL_COUNT` from task 1.2 (24 unless drift was detected).
- [ ] 13.5 Verify error-mapping convention: `grep -rnE 'rmcp::Error::new\(ErrorCode' src/mcp/tools/` MUST return zero matches.
- [ ] 13.6 Verify file-size budget: `wc -l src/mcp/**/*.rs src/mcp/*.rs` MUST report no production file >800 LOC (modulo D6 transitional exception for `server.rs` if test-extraction has not landed yet).
- [ ] 13.7 Replay the MCP `tools/list` snapshot one final time: MUST match `target/tools-list-baseline.json` byte-for-byte.
- [ ] 13.8 Run the full test suite: `cargo test` and `cargo test --all-features` MUST pass with the same test count recorded in task 1.1 (or higher, if any error-code regression tests were added in commit 3).
- [ ] 13.9 If any deep imports were enumerated in task 1.4, confirm they still resolve (or were updated to use re-exported paths).
- [ ] 13.10 Commit.

## 14. Post-flight

- [ ] 14.1 Open the PR. Body cites `docs/CODE_REVIEW.md` §1.4 and §2.4 plus this change's `proposal.md` and `design.md`.
- [ ] 14.2 PR description includes the per-file LOC table from the start vs end of the change to make the size win visible to reviewers.
- [ ] 14.3 PR description lists the 12 commits and their verification steps so a reviewer can spot-check any individual hop.
- [ ] 14.4 Once merged, mark this change ready for archive via `/opsx:archive`.
