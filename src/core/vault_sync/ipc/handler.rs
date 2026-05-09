//! Per-stream IPC handler and accept loop.
//!
//! The connection accept loop (`accept_ipc_clients`) and the
//! per-stream request handler (`handle_ipc_client`) still live in
//! `vault_sync::mod` because they reach into the runtime registry
//! (`PROCESS_REGISTRIES.dedup`), the per-session reload generation
//! counter, the in-flight handler-thread cap (`IPC_HANDLER_LIMIT`,
//! `IpcHandlerGuard`), and the watcher / supervisor handles. Moving
//! them here requires either widening every one of those helpers to
//! `pub(super)` or moving them along with the loop, and is deferred
//! to a follow-up commit.
//!
//! This file exists today so the directory layout invariant from
//! `vault-sync-module-layout` is satisfied (`ipc/handler.rs` exists)
//! and so a future contributor adding a new IPC handler has an
//! obvious home for it.

#![cfg(unix)]
