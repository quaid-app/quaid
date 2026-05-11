# mcp-server-module-layout Specification

## Purpose
TBD - created by archiving change decompose-mcp-server-module. Update Purpose after archive.
## Requirements
### Requirement: mcp is a directory module with a fixed submodule layout

The `crate::mcp` module SHALL be implemented as a directory module at `src/mcp/` containing the following files, each owning a single concern:

| File | Owns |
|---|---|
| `mod.rs` | module-level `//!` doc + re-exports |
| `server.rs` | `QuaidServer` struct, `ServerHandler` impl, bootstrap |
| `errors.rs` | every `map_*_error` helper + JSON-RPC error code constants |
| `validation.rs` | `validate_slug`, `validate_token`, `validate_temporal_value` |
| `tools/mod.rs` | `//!` doc + `pub mod` per domain |
| `tools/pages.rs` | `memory_get`, `memory_put`, `memory_list`, `memory_raw` |
| `tools/search.rs` | `memory_query`, `memory_search` |
| `tools/links.rs` | `memory_link`, `memory_link_close`, `memory_backlinks`, `memory_graph` |
| `tools/conversation.rs` | `memory_add_turn`, `memory_close_session`, `memory_close_action`, `memory_correct`, `memory_correct_continue` |
| `tools/assertions.rs` | `memory_check` |
| `tools/tags.rs` | `memory_tags`, `memory_timeline` |
| `tools/gaps.rs` | `memory_gap`, `memory_gaps` |
| `tools/admin.rs` | `memory_stats`, `memory_collections`, `memory_namespace_create`, `memory_namespace_destroy` |

A single-file `src/mcp/server.rs` containing tool method bodies SHALL NOT exist. New `#[tool]` methods SHALL be placed in the existing `tools/<domain>.rs` whose concern matches the tool's purpose; if no domain matches, a new `tools/<domain>.rs` SHALL be added rather than overloading an existing file or re-growing `server.rs`.

#### Scenario: server.rs no longer holds tool bodies

- **WHEN** the change lands
- **THEN** `src/mcp/server.rs` exists
- **AND** `src/mcp/server.rs` contains zero `#[tool(description = "..."]` attributes
- **AND** `src/mcp/server.rs` contains exactly one `impl QuaidServer { … }` block (for non-tool methods) and the `ServerHandler` impl
- **AND** the tool method bodies are distributed across `src/mcp/tools/*.rs`

#### Scenario: Required submodules exist after the change

- **WHEN** the change lands
- **THEN** `src/mcp/mod.rs`, `src/mcp/server.rs`, `src/mcp/errors.rs`, `src/mcp/validation.rs`, `src/mcp/tools/mod.rs` all exist as files
- **AND** the eight files `src/mcp/tools/{pages,search,links,conversation,assertions,tags,gaps,admin}.rs` all exist
- **AND** every tool listed in the table above is defined inside the file the table assigns it to

#### Scenario: A future tool lands in a domain-matched submodule

- **WHEN** a future change adds a new `#[tool]` method to `QuaidServer`
- **THEN** the method is defined inside the `src/mcp/tools/<domain>.rs` whose concern matches the tool's purpose
- **AND** if no existing domain matches, a new `src/mcp/tools/<domain>.rs` is created with a one-paragraph `//!` doc rather than placing the method in `server.rs` or overloading an unrelated domain file

### Requirement: Files under src/mcp/ obey an 800-LOC per-file budget

No file under `src/mcp/` SHALL exceed 800 lines as counted by `wc -l` (including blank lines and comments). This budget applies to production code files. Inline `#[cfg(test)] mod tests` blocks under `src/mcp/` SHALL also fit within the same budget; tests that would push a file past 800 LOC SHALL be moved to `tests/mcp_*.rs` instead of growing the file.

#### Scenario: Initial split holds the budget

- **WHEN** the change lands
- **THEN** `wc -l` reports no file under `src/mcp/` with more than 800 lines

#### Scenario: A future change cannot grow a file past the budget without re-splitting

- **WHEN** a future edit would push any file under `src/mcp/` past 800 lines
- **THEN** the change instead extracts a new submodule (or moves test code to `tests/`) so the budget is preserved

### Requirement: Every file under src/mcp/ has a module-level doc comment

Every `.rs` file under `src/mcp/` SHALL begin with a `//!` module-level doc comment of at least one paragraph that describes the file's concern. The doc comment SHALL identify which tools (if any) the file owns and how the file relates to the rest of `src/mcp/`.

#### Scenario: All mcp files document their purpose

- **WHEN** the change lands
- **THEN** every file under `src/mcp/` whose path matches `*.rs` begins with a non-empty `//!` block of at least one full sentence
- **AND** for every `tools/<domain>.rs`, the `//!` block names every tool that the file owns

### Requirement: All MCP errors route through mcp::errors helpers

Inside any `#[tool]` method body under `src/mcp/tools/`, the only permitted construction of `rmcp::Error` SHALL be a call to a helper function defined in `src/mcp/errors.rs` (`map_db_error`, `map_search_error`, `map_vault_sync_error`, `map_anyhow_error`, or future named helpers). Direct construction via `rmcp::Error::new(ErrorCode(...), ...)` SHALL NOT appear inside any `#[tool]` method body.

The helpers themselves are exempted: `src/mcp/errors.rs` may construct `rmcp::Error` directly because its sole purpose is the error-mapping convention. `src/mcp/validation.rs` is also exempted because validators emit `ErrorCode(-32602)` invalid-params errors as a control-flow primitive that predates the helper convention.

#### Scenario: No ad-hoc error construction in tool bodies

- **WHEN** the change lands
- **THEN** `grep -rnE "rmcp::Error::new\(ErrorCode" src/mcp/tools/` returns zero matches
- **AND** every `Result<CallToolResult, rmcp::Error>` returned from a `#[tool]` method body either propagates an `rmcp::Error` from a helper or constructs one via a helper call

#### Scenario: Error code values are unchanged across the audit

- **WHEN** a tool body that previously emitted `ErrorCode(-32003)` ad-hoc is rewritten to call `map_db_error`
- **THEN** the resulting `rmcp::Error` carries error code `-32003`
- **AND** integration tests that assert on specific error codes (`-32001`, `-32002`, `-32003`, `-32009`, `-32602`) continue to pass without modification

### Requirement: MCP wire surface is preserved across the split

The MCP wire surface SHALL be byte-for-byte preserved by this change. The set of tool names returned by the MCP `tools/list` request, the JSON Schema of each tool's input, the JSON Schema of each tool's output, and the JSON-RPC error codes emitted by each tool SHALL be identical before and after the change. The change SHALL NOT add, rename, remove, or modify any MCP tool.

#### Scenario: tools/list returns the same 24 tools

- **WHEN** an MCP client issues a `tools/list` request against the post-split server
- **THEN** the response contains exactly the 24 tools `memory_get`, `memory_put`, `memory_list`, `memory_raw`, `memory_query`, `memory_search`, `memory_link`, `memory_link_close`, `memory_backlinks`, `memory_graph`, `memory_check`, `memory_timeline`, `memory_tags`, `memory_gap`, `memory_gaps`, `memory_stats`, `memory_collections`, `memory_namespace_create`, `memory_namespace_destroy`, `memory_add_turn`, `memory_close_session`, `memory_close_action`, `memory_correct`, `memory_correct_continue`
- **AND** every tool's `description` string is identical to its pre-change value
- **AND** every tool's input JSON Schema is identical to its pre-change value

#### Scenario: Existing MCP integration tests pass unchanged

- **WHEN** the change lands
- **AND** the integration test suite under `tests/` is run without modification
- **THEN** every test that exercises an MCP tool by its public name passes
- **AND** no test required an edit to its `use` paths because `crate::mcp::QuaidServer` and the helpers re-exported from `crate::mcp::*` resolve to the same items as before

### Requirement: Public Rust surface of crate::mcp is preserved

The public surface of `crate::mcp` SHALL be byte-for-byte preserved by this change. Every public item (`pub fn`, `pub struct`, `pub enum`, `pub type`, `pub const`) that was reachable as `crate::mcp::Foo` before the split SHALL remain reachable at the same path after the split, via re-exports in `src/mcp/mod.rs` if its definition has moved into a submodule. No external `use crate::mcp::...` import in `src/`, `tests/`, or `benches/` SHALL require an edit as a result of this change.

#### Scenario: External call sites compile unchanged

- **WHEN** the change lands
- **AND** every existing `use crate::mcp::...` import outside `src/mcp/` is left as-is
- **THEN** `cargo build` succeeds with zero unresolved-import errors
- **AND** `cargo test` succeeds without any test-side import edits

#### Scenario: QuaidServer remains at its public path

- **WHEN** the change lands
- **THEN** `crate::mcp::QuaidServer` resolves to the same struct as before the change
- **AND** `crate::mcp::map_db_error`, `crate::mcp::map_search_error`, and other previously-public helpers resolve via re-exports in `src/mcp/mod.rs`
