# MCP Tool Rename Spec

**Change:** All 17 MCP tools renamed from the legacy prefix to the `memory_*` prefix.

## Current tool names (post-rename)

| New name |
|----------|
| `memory_get` |
| `memory_put` |
| `memory_query` |
| `memory_search` |
| `memory_list` |
| `memory_link` |
| `memory_link_close` |
| `memory_backlinks` |
| `memory_graph` |
| `memory_timeline` |
| `memory_tags` |
| `memory_check` |
| `memory_gap` |
| `memory_gaps` |
| `memory_stats` |
| `memory_raw` |
| `memory_collections` |

## Invariants

1. The `#[tool(name = "...")]` annotation in `src/mcp/server.rs` for each tool must use the `memory_*` name exactly as listed above.
2. The corresponding Rust method name should match the tool name for clarity (e.g., method `memory_get` for tool `memory_get`).
3. No legacy-prefix tool name appears in any `#[tool]` annotation in the final implementation.
4. Tool descriptions and input schema are unchanged — only the tool name changes.
5. Error messages that refer to a tool by name (e.g., "use `memory_put` to write") must use the new name.

## Validation

- A search for legacy-prefix tool name patterns in `src/mcp/server.rs` returns zero matches.
- MCP `tools/list` response from `quaid serve` must contain only `memory_*` tool names.
