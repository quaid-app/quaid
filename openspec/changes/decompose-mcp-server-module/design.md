## Context

`src/mcp/server.rs` is the MCP plumbing surface for Quaid. It currently holds **5,903 LOC** in one file, decomposing roughly as:

| Range | Concern | LOC |
|---|---|---|
| 1–37 | Imports + JSON-RPC error code constants | 37 |
| 38–356 | Validators (`validate_slug`, `validate_token`, `validate_temporal_value`) | ~320 |
| 357–546 | `map_*_error` helpers | ~190 |
| 547–818 | `QuaidServer` struct, `ServerHandler` impl, helper free functions | ~270 |
| 819–1995 | The `#[tool(tool_box)] impl QuaidServer { … }` block holding **24** `#[tool]` methods | ~1,176 |
| 1996–2015 | Misc tail (re-imports for tests) | ~20 |
| 2016–5903 | `#[cfg(test)] mod tests { … }` | 3,888 |

The `extract-inline-tests-to-integration` change (sibling, already proposed) will move the 3,888-line test block to `tests/mcp_server_*.rs`, leaving ~2,015 production lines. Even after that, one file owns:

1. trait/struct definitions for the server,
2. JSON-RPC error code constants and validators,
3. `map_*_error` adapters from internal error enums to `rmcp::Error`,
4. every `#[tool]` method body (24 methods),
5. `ServerHandler` plumbing and bootstrap.

The `#[tool(tool_box)]` macro from `rmcp` permits the impl block to be split — an `#[tool(tool_box)]` impl block can appear multiple times for the same struct, and the macro registers each method into the same tool list. This is the structural lever that makes domain grouping a pure cut/paste.

Constraints:

- **MCP wire surface is the contract.** Tool names (`memory_get`, `memory_put`, …), input schemas (derived from `MemoryGetInput` etc. via `schemars`), output shapes (`CallToolResult`), and JSON-RPC error codes (-32001, -32002, -32003, -32009, -32602) MUST be byte-identical before and after every commit.
- **No file >800 LOC.** This anticipates the file-size clippy lint that `add-rust-lints-and-ci-gate` will introduce.
- **Every commit builds and tests pass.** The split is mechanical and reversible at every step.
- **Intersects with `extract-inline-tests-to-integration`.** Both touch `src/mcp/server.rs`. The two changes are independent in either order: see *Decisions / D6*.

Stakeholders: anyone who imports `crate::mcp::*` (currently `src/main.rs` and `src/lib.rs` re-exports for tests). External crates do not link against this crate as a library; the only consumer is the binary itself and the integration test suite.

## Goals / Non-Goals

**Goals**

- Reduce `src/mcp/server.rs` from 2,015 production LOC to a thin module that contains only `QuaidServer`, `ServerHandler` impl, and bootstrap.
- Group the 24 tool methods into eight domain-named files under `src/mcp/tools/`, each <800 LOC.
- Lift the 190-line `map_*_error` block to `src/mcp/errors.rs`. Every tool body's error path goes through one of these helpers; ad-hoc `rmcp::Error::new(ErrorCode(...), ...)` is forbidden in tool bodies.
- Lift the 320-line validators block to `src/mcp/validation.rs`.
- Each new file gets a `//!` module-level paragraph.
- Preserve the public Rust surface (`crate::mcp::QuaidServer` etc.) byte-for-byte via re-exports.
- Preserve the MCP wire surface (tool list, schemas, error codes) byte-for-byte.

**Non-Goals**

- No behavioural changes to any tool. No new tools, no new error codes, no schema edits, no rename of any public item.
- No refactor of tool bodies beyond the error-mapping audit. If a tool calls `db.lock().unwrap_or_else(|e| e.into_inner())`, it keeps doing so. Internal control flow inside each tool body is preserved.
- No move of the `#[cfg(test)] mod tests` block — that is the scope of `extract-inline-tests-to-integration`. This change leaves the inline test block where it is and ensures it still compiles against the re-exported surface (or, if the test extraction lands first, this change updates the relocated `tests/mcp_server_*.rs` import paths only).
- No change to the validators' signatures or semantics. Pure cut/paste.
- No change to `MemoryGetInput`, `MemoryPutInput`, etc. — input/output structs continue to live where they are (most are in `src/mcp/inputs.rs` or similar; verify during execution).
- No change to the rmcp dependency or the `#[tool(...)]` macro invocation pattern.

## Decisions

### D1. Module layout

Final layout (as locked into the proposal and the spec):

```text
src/mcp/
├── mod.rs           — re-exports + module-level //! doc
├── server.rs        — QuaidServer struct, ServerHandler impl, bootstrap
├── errors.rs        — every map_*_error helper + JSON-RPC error code constants
├── validation.rs    — validate_slug / validate_token / validate_temporal_value
└── tools/
    ├── mod.rs       — //! doc + pub mod each domain file
    ├── pages.rs     — memory_get, memory_put, memory_list, memory_raw
    ├── search.rs    — memory_query, memory_search
    ├── links.rs     — memory_link, memory_link_close, memory_backlinks, memory_graph
    ├── conversation.rs — memory_add_turn, memory_close_session,
    │                    memory_close_action, memory_correct, memory_correct_continue
    ├── assertions.rs — memory_check
    ├── tags.rs      — memory_tags, memory_timeline
    ├── gaps.rs      — memory_gap, memory_gaps
    └── admin.rs     — memory_stats, memory_collections,
                       memory_namespace_create, memory_namespace_destroy
```

Tool counts per file (from grep verification, see *D5*):

| File | Tool methods | Notes |
|---|---|---|
| `tools/pages.rs` | 4 | `memory_get`, `memory_put`, `memory_list`, `memory_raw` |
| `tools/search.rs` | 2 | `memory_query`, `memory_search` |
| `tools/links.rs` | 4 | `memory_link`, `memory_link_close`, `memory_backlinks`, `memory_graph` |
| `tools/conversation.rs` | 5 | `memory_add_turn`, `memory_close_session`, `memory_close_action`, `memory_correct`, `memory_correct_continue` (plus the `memory_close_action_impl` helper which is not a `#[tool]` but co-locates) |
| `tools/assertions.rs` | 1 | `memory_check` |
| `tools/tags.rs` | 2 | `memory_tags`, `memory_timeline` |
| `tools/gaps.rs` | 2 | `memory_gap`, `memory_gaps` |
| `tools/admin.rs` | 4 | `memory_stats`, `memory_collections`, `memory_namespace_create`, `memory_namespace_destroy` |
| **Total** | **24** | matches verified count |

**Rationale.** Domain grouping by behaviour minimises cross-file dependencies: pages are CRUD on a slug, search reads pages, links connect pages, conversation handles session lifecycle, and admin is wire-frame. The largest projected file is `tools/pages.rs` (memory_put alone is ~95 LOC; total domain ~270 LOC), well under 800.

**Alternatives considered**:

- *Single `tools/mod.rs` with all 24 methods.* Rejected: would still be ~1,200 LOC and re-create the original problem one level down.
- *One file per tool (24 files).* Rejected: too granular; cross-cutting helpers (collection guards, namespace resolution) get duplicated or trampled by `pub(crate) use`. Domain grouping mirrors the conceptual MCP API the user already navigates.
- *Group by error type instead of domain.* Rejected: a tool's error space is mostly a function of its inputs, not its outputs. Domain grouping is what humans read for.

### D2. `#[tool(tool_box)]` across multiple `impl` blocks

`rmcp`'s `#[tool(tool_box)]` macro emits the tool registration via the impl block's expansion. Each `impl` block annotated `#[tool(tool_box)]` registers the methods inside it independently. The crate's existing usage at lines 819 and 1995 already shows two `#[tool(tool_box)]` annotations — one on the giant impl block and one on the empty trailing block — confirming multiple-impl-block support.

Each new `tools/<domain>.rs` will contain exactly one:

```rust
#[tool(tool_box)]
impl QuaidServer {
    #[tool(description = "…")]
    pub fn memory_xxx(&self, #[tool(aggr)] input: …) -> Result<CallToolResult, rmcp::Error> {
        …
    }
    // …
}
```

**Verification of equivalence.** After the split, the MCP `tools/list` response MUST contain the same 24 tool entries with the same descriptions and the same JSON Schemas. This is verified by an integration test that calls `tools/list` against a freshly-bootstrapped server and snapshots the result. (See *Tasks → Verification gates*.)

**Risk.** If `#[tool(tool_box)]` panics or duplicates registrations when the same struct is targeted by multiple impl blocks across files, the move would not be a pure cut/paste. *Mitigation*: this is verified empirically by the very first per-domain commit — extract `tools/admin.rs` first because it is the smallest and most isolated (4 methods, no cross-cutting state); if `tools/list` no longer matches expected output after that commit, abort and revert before touching anything else. (See *Risks / R1*.)

### D3. Error-mapping audit (§2.4 ride-along)

`grep -nE "rmcp::Error::new\(ErrorCode" src/mcp/server.rs` reports **24 call sites** in tool method bodies that construct an `rmcp::Error` ad-hoc, plus the legitimate uses inside the helpers themselves at lines 357–546 and 39 (validators).

Categorisation of the 24 ad-hoc sites:

- **Mappable to `map_db_error`**: 22 sites that wrap a database-layer error with `format!("database error: {e}")` or `e.to_string()` and `ErrorCode(-32003)`. Verified by reading each call site context (`.map_err(|e| rmcp::Error::new(ErrorCode(-32003), e.to_string(), None))?` typically follows a rusqlite or db-layer call).
- **Mappable to `map_anyhow_error` / a serialisation helper**: 2 sites in `memory_collections` / `memory_stats` that wrap `serde_json::to_string_pretty(...).unwrap_or…` with `ErrorCode(-32003)`.

**Decision**: every ad-hoc construction in a tool body is replaced with the appropriate `map_*_error` helper. If two helpers nearly duplicate (`map_db_error` vs the `e.to_string()` variant), prefer the more structured helper. If a call site formats a JSON serialisation failure, introduce a new `mcp::errors::map_serialize_error` helper rather than reaching for `rmcp::Error::new` — this is one new helper, not a structural change.

**Permitted ad-hoc sites after the audit**: only inside `mcp/errors.rs` (helper bodies) and `mcp/validation.rs` (validators emit `ErrorCode(-32602)` invalid-params errors directly because they predate the helpers). Validators are intentionally not folded into `errors.rs` because they are control flow, not error mapping.

**Alternatives considered**:

- *Skip the audit and do it as a follow-up.* Rejected: the proposed file moves touch every tool body anyway. Auditing while moving is materially cheaper than a second pass over eight files later. The CODE_REVIEW explicitly recommends bundling them.
- *Add a clippy lint that rejects `rmcp::Error::new` outside `mcp/errors.rs`.* Deferred: belongs to `add-rust-lints-and-ci-gate`. This change establishes the convention; the lint enforces it later.

### D4. Commit sequence

The execution sequence is constrained by the requirement that every commit builds and the MCP wire surface is preserved at every commit boundary:

1. **Commit 1 — `mcp/errors.rs`**: cut the entire `map_*_error` block (lines 357–546) and the JSON-RPC error code constants from `server.rs`. Place in `src/mcp/errors.rs` with a `//!` doc. Re-export from `mcp/mod.rs` so existing `crate::mcp::map_db_error` paths still work. Verify: `cargo build && cargo test`.
2. **Commit 2 — `mcp/validation.rs`**: cut the `validate_*` block (lines 38–356). Same re-export pattern. Verify: `cargo build && cargo test`.
3. **Commit 3 — Error-mapping audit**: rewrite the 24 ad-hoc `rmcp::Error::new(ErrorCode(...), ...)` sites in tool bodies to call `map_*_error` helpers. If a `map_serialize_error` helper is needed (D3), introduce it in `mcp/errors.rs` in this commit. Verify: `cargo build && cargo test`. The MCP `tools/list` snapshot test must still pass — error code values must not change.
4. **Commit 4 — `mcp/tools/admin.rs`**: smallest, most isolated domain. Establishes the multi-`impl`-block pattern. Verify: `cargo build && cargo test && cargo run -- (tools/list smoke test)`.
5. **Commits 5–11 — one per remaining domain**, in increasing complexity order: `tags.rs` → `gaps.rs` → `assertions.rs` → `links.rs` → `search.rs` → `conversation.rs` → `pages.rs`. Each commit moves the methods, leaves NO `// moved to tools/<domain>.rs` marker — the original arguments suggested marker-and-clean-up commits, but markers are stale-on-arrival comments and `git log` already records the move. Verify each commit: `cargo build && cargo test`.
6. **Commit 12 — Final cleanup**: ensure `server.rs` contains only `QuaidServer`, `ServerHandler` impl, bootstrap. Add `//!` doc paragraphs to every new file. Verify: `cargo build && cargo test && wc -l src/mcp/**/*.rs` (no file >800 LOC) `&& grep -nE "rmcp::Error::new\(ErrorCode" src/mcp/tools/` (zero matches outside helpers).

The original arguments suggested a marker-comment-then-removal pattern across two commits per domain. **Rejected** in favour of a single commit per domain: the marker only adds noise and risks landing if the cleanup commit is forgotten. Git history is the source of truth for "where did this method go".

### D5. Verified tool count is 24, not 26

`grep -cE "^\s*#\[tool\(description" src/mcp/server.rs` returns **24**. `docs/CODE_REVIEW.md` §1.4 says 26. The discrepancy is a documentation drift in the review; the actual `impl QuaidServer` contains:

```text
memory_get          memory_link              memory_stats
memory_put          memory_link_close        memory_collections
memory_list         memory_backlinks         memory_namespace_create
memory_raw          memory_graph             memory_namespace_destroy
memory_query        memory_check             memory_add_turn
memory_search       memory_timeline          memory_close_session
memory_link         memory_tags              memory_close_action
                    memory_gap               memory_correct
                    memory_gaps              memory_correct_continue
```

That is 24 distinct tools. The split's per-domain file allocations sum to 24.

**Verification step in tasks**: run `grep -cE "^\s*#\[tool\(description" src/mcp/tools/**/*.rs` after the final commit; the result MUST equal 24 (or the value `grep -cE "^\s*#\[tool\(description" src/mcp/server.rs` returned at the start of the change, whichever is higher — to defend against new tools being added during the work).

### D6. Interaction with `extract-inline-tests-to-integration`

Both this change and `extract-inline-tests-to-integration` touch `src/mcp/server.rs`. They are independent in either order:

- **If this change lands first**: the inline `mod tests` block at lines 2016–5903 stays in `server.rs`, but `server.rs` is now ~270 LOC of bootstrap + ~3,888 LOC of tests = 4,158 LOC. This violates the 800-LOC rule for production files but is acceptable transitionally because (a) the `mod tests` block is `#[cfg(test)]` so it does not ship in the production binary and (b) the lint gate from `add-rust-lints-and-ci-gate` does not yet exist. The follow-on `extract-inline-tests-to-integration` change will move the test block to `tests/mcp_server_*.rs` and bring `server.rs` to its final ~270-LOC shape.
- **If `extract-inline-tests-to-integration` lands first**: it relocates the test block to `tests/mcp_server_*.rs` against a still-monolithic `mcp/server.rs`. When this change then lands, the relocated tests' imports may need updating (`use quaid::mcp::map_db_error` → still works via re-export, no edit needed; `use quaid::mcp::server::QuaidServer` → still works via re-export, no edit needed). Imports that reach into private items will need updating, but those tests should not exist in `tests/` (white-box tests stay inline per `docs/CODE_REVIEW.md` §1.5).

Either ordering is supported. The change author picks one based on which sibling change is closer to apply-ready.

### D7. Re-exports preserve `crate::mcp::*`

`src/mcp/mod.rs` will re-export every public item that was previously `pub` from `src/mcp/server.rs`:

```rust
//! MCP (Model Context Protocol) stdio server. The QuaidServer struct, its
//! ServerHandler impl, and bootstrap live in `server`. Tool method bodies
//! are grouped by domain under `tools/`. Validators live in `validation`,
//! error mappers in `errors`. The MCP wire surface is the public contract;
//! Rust-level paths (`crate::mcp::QuaidServer`, etc.) are preserved via
//! re-exports here.

pub mod errors;
pub mod server;
pub mod tools;
pub mod validation;

pub use errors::*;       // map_db_error, map_search_error, etc.
pub use server::QuaidServer;
pub use validation::{validate_slug, validate_token, validate_temporal_value};
```

The blanket `pub use errors::*` is acceptable here because `errors.rs` is a curated set of helper functions — adding a new helper in the future does not surprise consumers because consumers explicitly want every `map_*_error` to be available.

## Risks / Trade-offs

- **R1: `#[tool(tool_box)]` may not support multiple impl blocks across files.** → *Mitigation*: the smallest domain (`admin.rs`, 4 methods) is moved first as a probe. If the `tools/list` snapshot test fails after that commit, revert and re-evaluate before touching anything else. Empirical evidence from the existing codebase (two `#[tool(tool_box)]` annotations already exist at lines 819 and 1995) suggests this works, but the probe formalises the verification.
- **R2: A tool body's helper functions or constants are private to `server.rs` and break when moved.** → *Mitigation*: when moving each domain, run `cargo build` immediately. Any unresolved-import error names a private item that must be promoted to `pub(crate)` and re-exported. This is local and additive — no external API change. Common candidates: `MAX_LIMIT`, namespace-resolution helpers, collection-guard predicates.
- **R3: The error-mapping audit changes an error code by accident.** → *Mitigation*: a JSON snapshot test of `tools/list` does not catch this — error codes are emitted at runtime, not in the schema. Add a regression test that exercises one error path per code value (-32001, -32002, -32003, -32009, -32602) before commit 3 lands. The existing test suite already covers most paths; verify coverage with `cargo test 2>&1 | grep -i 'returns_.*_error_code'` and add only what's missing.
- **R4: Conflict with `extract-inline-tests-to-integration` if both land near each other.** → *Mitigation*: see D6. Either order works; the second change to land does a one-line rebase on top of the first.
- **R5: `pub use errors::*` accidentally re-exports something internal.** → *Mitigation*: at the end of commit 1, audit `errors.rs` for any `pub fn` that should actually be `pub(crate)`. Anything used only within `mcp/` becomes `pub(crate)` and is dropped from the blanket re-export.
- **R6: A future tool added to `server.rs` (drift) lands during the change.** → *Mitigation*: `D5` verification step compares the tool count at start to the tool count at end. Any drift is surfaced and routed to its domain file before the change closes.
- **R7: 800-LOC budget is violated by `tools/pages.rs` because `memory_put` is unusually long.** → *Tradeoff*: `memory_put` is ~95 LOC alone (`server.rs:1091–1183`); `memory_get` ~40 LOC; `memory_list` ~75 LOC; `memory_raw` ~65 LOC. Total ~275 LOC plus imports and tests-side helpers. Comfortably under 800. If a future change pushes `tools/pages.rs` past 800, the right response is another split (e.g. extract `tools/pages/raw.rs`), not raising the budget.

## Migration Plan

This is a pure code-move. There is no data migration, no on-disk format change, and no MCP-client-visible change. The deployment story is:

1. Land commits 1–12 in order.
2. After each commit, CI runs `cargo build`, `cargo test`, and the `tools/list` integration test. Any failure aborts the merge.
3. No rollback procedure is required at the user-data level. To revert: `git revert` the offending commit.
4. The MCP server continues to serve clients across the change without any version bump or schema migration.

## Open Questions

- **Q1: Does the `#[tool(tool_box)]` macro emit a single tool registry or one per impl block?** Verified empirically by commit 4 (the `admin.rs` probe). If it emits per-impl-block registries that aren't merged, the design is wrong and we revert.
- **Q2: Are there any existing call sites in non-`mcp/` code that import deep paths like `crate::mcp::server::map_db_error`?** Run `grep -rE "use crate::mcp::server::" src/` to enumerate before commit 1. Any deep-path import must be updated to the new re-exported path. Expected: zero or one or two; this is a small repo and the convention is to import via `crate::mcp::*`.
- **Q3: Should `tools/conversation.rs` co-locate the private helper `memory_close_action_impl` (currently `server.rs:982`)?** Yes — it is the implementation core of `memory_close_action` and only that tool calls it. Move it alongside in the same commit. This also keeps the helper out of the public re-export surface.
