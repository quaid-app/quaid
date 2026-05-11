//! Quaid — personal AI memory backed by SQLite, FTS5, and a local
//! sentence-embedding model. The crate is the library half of the `quaid`
//! binary: it owns the on-disk store, the search and retrieval stack, the
//! conversation extraction pipeline, the vault-sync engine, and the MCP
//! tool surface that consumers (Claude Code and other MCP clients) drive
//! over stdio JSON-RPC.
//!
//! Top-level layout:
//! - [`core`] — library internals. Database connection and schema
//!   ([`core::db`]), shared types ([`core::types`]), markdown parsing,
//!   FTS5 and vector search, hybrid retrieval, the
//!   [`core::conversation`] extraction pipeline, the
//!   [`core::vault_sync`] live-sync engine, link graph, assertions, and
//!   knowledge-gap tracking.
//! - [`mcp`] — the MCP server. [`mcp::server::QuaidServer`] holds the
//!   request handler; the individual `memory_*` tools live in
//!   [`mcp::tools`]; cross-cutting error mapping and validation live in
//!   [`mcp::errors`] and [`mcp::validation`].
//! - [`commands`] — CLI command dispatch backing the `quaid` binary. Not
//!   considered part of the public API surface (its contract is the
//!   `clap`-generated `--help` text), and intentionally exempt from this
//!   crate's `missing_docs` lint.
//!
//! Where to start reading:
//! - **Library consumers** (people calling Quaid as an MCP server)
//!   should start at [`mcp::server`] for the tool surface and
//!   [`core::conversation`] for how turns become extracted pages.
//! - **Codebase maintainers** should start at [`core::db`] for the
//!   schema and connection lifecycle, then [`core::vault_sync`] for the
//!   liveness-and-restore model that ties the store to a working tree.
//!
//! Agent-facing workflow documentation lives in `skills/*/SKILL.md` and
//! is intentionally not part of the rustdoc surface: those files are
//! prompts for AI agents, not documentation for human callers.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::print_stdout,
        reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
    )
)]
#![warn(missing_docs)]

#[allow(
    missing_docs,
    reason = "CLI command dispatch — user-facing contract is clap --help text, not rustdoc; see spec public-api-docs Requirement: Documentation scope boundary"
)]
pub mod commands;
pub mod core;
pub mod mcp;
