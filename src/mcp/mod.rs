//! MCP (Model Context Protocol) stdio server. The `QuaidServer` struct, its
//! `ServerHandler` impl, and bootstrap live in `server`. Tool method bodies
//! are grouped by domain under `tools/`. Validators live in `validation`,
//! error mappers in `errors`. The MCP wire surface is the public contract;
//! Rust-level paths (`crate::mcp::QuaidServer`, etc.) are preserved via
//! re-exports here.

pub mod errors;
pub mod http;
pub mod server;
pub mod tools;
pub mod validation;

pub use errors::*;
pub use http::{
    bind_with_token_guard, build_connection_service, run_http, HttpConfig, HttpConfigError,
};
pub use server::QuaidServer;
pub use validation::{validate_slug, validate_temporal_value, validate_token};
