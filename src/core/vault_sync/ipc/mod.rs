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
//! `vault_sync` and only used by the socket helpers in
//! [`socket`] that bind / publish the socket.
//!
//! The connection accept loop ([`handler::accept_ipc_clients`])
//! and per-stream request handler live in
//! [`handler`]. The supervisor in `vault_sync::mod` calls
//! `accept_ipc_clients` each tick and the handler runs per
//! connection on its own thread.

pub(super) mod handler;
pub(super) mod socket;

#[cfg(unix)]
pub(super) use handler::accept_ipc_clients;
#[cfg(all(test, unix))]
pub(super) use handler::{IpcHandlerGuard, IPC_HANDLER_LIMIT};
#[cfg(all(test, unix, target_os = "linux"))]
pub(super) use socket::audit_bound_ipc_socket;
#[cfg(unix)]
pub(super) use socket::{cleanup_published_ipc_socket, publish_ipc_socket};

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

/// Errors surfaced by the IPC subsystem and wrapped by
/// [`super::VaultSyncError::Ipc`] at the API boundary.
#[cfg(unix)]
#[derive(Debug, Error)]
pub enum IpcError {
    /// Raised when the directory that will host the IPC socket has
    /// permissions or ownership the server refuses to bind under.
    #[error("IpcDirectoryInsecureError: path={path} reason={reason}")]
    IpcDirectoryInsecure {
        /// Filesystem path of the offending directory.
        path: String,
        /// Human-readable description of what about the directory
        /// failed the security check.
        reason: String,
    },

    /// Raised when the on-disk IPC socket has permissions the server
    /// refuses to publish or the client refuses to connect to.
    #[error("IpcSocketPermissionError: path={path} reason={reason}")]
    IpcSocketPermission {
        /// Filesystem path of the IPC socket.
        path: String,
        /// Human-readable description of the permission mismatch.
        reason: String,
    },

    /// Raised when a leftover or third-party file blocks publishing
    /// the IPC socket at its expected path.
    #[error("IpcSocketCollisionError: path={path} reason={reason}")]
    IpcSocketCollision {
        /// Filesystem path of the conflicting file.
        path: String,
        /// Human-readable description of the collision.
        reason: String,
    },

    /// Raised when the connected peer's credentials (uid/pid) do not
    /// authorise it to talk to this serve session.
    #[error("IpcPeerAuthFailedError: path={path} reason={reason}")]
    IpcPeerAuthFailed {
        /// Filesystem path of the IPC socket the peer connected on.
        path: String,
        /// Human-readable description of the auth failure.
        reason: String,
    },
}
