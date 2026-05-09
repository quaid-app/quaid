//! IPC datagram types and the IPC error enum.
//!
//! `IpcError` is the per-subsystem child enum surfaced through
//! `VaultSyncError::Ipc` for IPC-specific authentication and
//! socket-permission failures.
//!
//! Wire types — [`IpcRequest`], [`IpcResponse`] — describe the
//! JSON envelope exchanged between a CLI client and a serve
//! process over the per-session unix socket. They are
//! `pub(crate)` because both `src/commands/put.rs` (the client
//! side) and the serve handler in `vault_sync::mod` need to
//! construct and pattern-match on them, but they are not part of
//! the external `quaid::core::vault_sync` public surface.
//!
//! [`IpcPeerCredentials`] is the OS-level peer identity returned
//! by `socket::peer_credentials_for_stream`.
//! [`LiveServeEndpoint`] is the row returned by
//! `live_serve_endpoint_for_root_path` when a serve session is
//! live for a given vault root — both the CLI and the watcher
//! use it to decide whether to talk over IPC or fall back to
//! direct writes.
//!
//! `IpcSocketLocation` and `PublishedIpcSocket` are the on-disk
//! placement and the bound listener; they are private to
//! `vault_sync` and only used by the runtime helpers in
//! `vault_sync::mod` that bind / publish the socket.
//!
//! The connection accept loop (`accept_ipc_clients`) and per-stream
//! request handler (`handle_ipc_client`) still live in
//! `vault_sync::mod` because they reach into `start_serve_runtime`'s
//! private state (PROCESS_REGISTRIES.dedup,
//! IpcHandlerGuard, IPC_HANDLER_LIMIT, the per-session reload
//! generation counter, etc.). They will move to
//! `ipc::handler` in a follow-up commit.

pub(super) mod socket;

#[cfg(unix)]
use std::os::unix::net::UnixListener;
#[cfg(unix)]
use std::path::PathBuf;

#[cfg(unix)]
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use thiserror::Error;

#[cfg(unix)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IpcPeerCredentials {
    pub pid: i32,
    pub uid: u32,
}

#[cfg(unix)]
#[derive(Debug, Clone)]
pub(crate) struct LiveServeEndpoint {
    pub session_id: String,
    pub pid: i64,
    pub ipc_path: String,
}

#[cfg(unix)]
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum IpcRequest {
    WhoAmI,
    Put {
        slug: String,
        content: String,
        expected_version: Option<i64>,
    },
}

#[cfg(unix)]
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum IpcResponse {
    WhoAmI { session_id: String },
    PutOk { status: String },
    Error { error: String },
}

#[cfg(unix)]
pub(super) struct PublishedIpcSocket {
    pub(super) listener: UnixListener,
    pub(super) path: PathBuf,
}

#[cfg(unix)]
pub(super) struct IpcSocketLocation {
    pub(super) runtime_root: PathBuf,
    pub(super) socket_dir: PathBuf,
    pub(super) create_runtime_root: bool,
}

#[cfg(unix)]
#[derive(Debug, Error)]
pub enum IpcError {
    #[error("IpcDirectoryInsecureError: path={path} reason={reason}")]
    IpcDirectoryInsecure { path: String, reason: String },

    #[error("IpcSocketPermissionError: path={path} reason={reason}")]
    IpcSocketPermission { path: String, reason: String },

    #[error("IpcSocketCollisionError: path={path} reason={reason}")]
    IpcSocketCollision { path: String, reason: String },

    #[error("IpcPeerAuthFailedError: path={path} reason={reason}")]
    IpcPeerAuthFailed { path: String, reason: String },
}
