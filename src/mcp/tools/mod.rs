//! Domain-grouped MCP tool method bodies. Each submodule contributes a
//! single `#[tool(tool_box)] impl QuaidServer` block hosting the tools
//! whose concern matches the file name. The `#[tool(tool_box)]` macro
//! supports multiple impl blocks per struct, so the registry built from
//! all of these is identical to the pre-split single-block registry.
//! See `openspec/changes/decompose-mcp-server-module/design.md` for the
//! exact tool-to-file allocation.

pub mod admin;
pub mod assertions;
pub mod gaps;
pub mod links;
pub mod tags;
