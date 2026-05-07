## Why

`src/mcp/server.rs` is 5,903 LOC — the largest production file in the crate. Even after the planned `extract-inline-tests-to-integration` work moves the 3,888-line `mod tests` block to `tests/mcp_server_*.rs`, the remaining ~2,015 production lines still hold a single `impl QuaidServer { … }` block (lines 820–1995) with **24** `#[tool]` methods, plus 200 lines of `map_*_error` adapters (357–546) and 320 lines of validators (38–356). One file does too many jobs: trait/struct definitions, validation, error mapping, every MCP tool body, and bootstrap. Per `docs/CODE_REVIEW.md` §1.4 the `#[tool]` macro permits multiple `impl` blocks, so domain grouping is a pure cut/paste with re-exports — there is no behavioural risk gating this work.

Riding along is `docs/CODE_REVIEW.md` §2.4: at least 24 call sites (e.g. `server.rs:1806–1807`, `:1811`, `:1944`) construct `rmcp::Error::new(ErrorCode(-32003), …)` ad-hoc instead of routing through the `map_db_error` / `map_search_error` / `map_vault_sync_error` helpers that already exist at lines 357–546. Auditing this once, while we are already moving every tool body, is materially cheaper than auditing it later when the call sites are spread across eight files.

## What Changes

- Decompose `src/mcp/server.rs` into a `src/mcp/` module tree:
  - `mod.rs` — module root and re-exports.
  - `server.rs` — `QuaidServer` struct, `ServerHandler` impl, and bootstrap only. No tool methods.
  - `errors.rs` — every `map_*_error` helper currently at lines 357–546.
  - `validation.rs` — `validate_slug`, `validate_token`, `validate_temporal_value` and the JSON-RPC error code constants currently at lines 38–356.
  - `tools/pages.rs` — `memory_get`, `memory_put`, `memory_list`, `memory_raw`.
  - `tools/search.rs` — `memory_query`, `memory_search`.
  - `tools/links.rs` — `memory_link`, `memory_link_close`, `memory_backlinks`, `memory_graph`.
  - `tools/conversation.rs` — `memory_add_turn`, `memory_close_session`, `memory_close_action`, `memory_correct`, `memory_correct_continue`.
  - `tools/assertions.rs` — `memory_check`.
  - `tools/tags.rs` — `memory_tags`, `memory_timeline`.
  - `tools/gaps.rs` — `memory_gap`, `memory_gaps`.
  - `tools/admin.rs` — `memory_stats`, `memory_collections`, `memory_namespace_create`, `memory_namespace_destroy`.
- Audit and fix every ad-hoc `rmcp::Error::new(ErrorCode(...), ...)` construction inside tool method bodies. All errors must route through helpers in `mcp/errors.rs`. The known offenders are at `server.rs:1806–1807` and ~22 other `-32003` sites surfaced by `grep -nE "rmcp::Error::new\(ErrorCode" src/mcp/server.rs`. Permitted ad-hoc construction is limited to the helper bodies themselves in `errors.rs` and `validation.rs`.
- Every new file gets a one-paragraph `//!` module doc describing its scope.
- No file exceeds 800 LOC after the split.
- Public MCP wire surface is unchanged. Tool names, input/output schemas, and error codes are byte-identical before and after.

This is **not** a behavioural change. It is purely a structural reorganisation plus an error-mapping audit.

## Capabilities

### New Capabilities

- `mcp-server-module-layout`: structural and conventional invariants for the `src/mcp/` module — fixed submodule layout, 800-LOC per-file budget, mandatory `//!` module docs, single error-mapping convention (`mcp::errors::map_*` only — no ad-hoc `rmcp::Error::new(ErrorCode(...), ...)` outside helper bodies), and preservation of the MCP wire surface across the split. This capability locks in the post-split shape so that future changes (e.g. `add-rust-lints-and-ci-gate`'s file-size lint, `add-rust-lints-and-ci-gate`'s warn-on-missing-docs) operate against a stable target, and so that future tool additions go in a domain-matched submodule rather than re-growing `server.rs`.

### Modified Capabilities

None. No existing spec's REQUIREMENTS change. Every spec under `openspec/specs/` that exercises an MCP tool (for example `agent-writes`, `conversation-turn-capture`, `correction-dialogue`, `mcp-tool-rename`, `namespace-isolation`) continues to hold by construction because tool names, schemas, and error codes are unchanged. The reorganisation is invisible to MCP clients.

## Impact

- **Affected code**: `src/mcp/server.rs` is split into `src/mcp/{mod.rs, server.rs, errors.rs, validation.rs, tools/*.rs}`. No other production source file changes.
- **Tests**: the 3,888-line `mod tests` block at `server.rs:2016–5903` continues to compile against the re-exported public surface. If the inline-tests-extraction proposal lands first, the relocated `tests/mcp_server_*.rs` files need their `use quaid::mcp::…` imports updated to the new paths; if this proposal lands first, the inline `mod tests` block keeps working via `super::*` against the re-exports in `mcp/mod.rs`. Either ordering is supported.
- **Public APIs**: `QuaidServer` and its methods remain `pub` and remain reachable at the same import paths via re-exports from `src/mcp/mod.rs`.
- **MCP wire surface**: unchanged. `tools/list` returns the same 24 tools with the same names, descriptions, and JSON Schemas. Error codes (-32001, -32002, -32003, -32009, -32602) are unchanged.
- **Dependencies**: none added or removed.
- **Build**: a successful `cargo build` and `cargo test` is required at every commit boundary.
- **Note on tool count**: `docs/CODE_REVIEW.md` §1.4 says "26 #[tool] methods" but a literal count via `grep -cE '^\s*#\[tool\(description' src/mcp/server.rs` returns **24**. The design will use the verified count of 24 and treat any discrepancy surfaced during execution as a hard stop pending investigation.
