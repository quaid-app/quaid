//! Peer-credential / per-session authorisation and socket
//! placement helpers for the IPC channel (`cfg(unix)` only).
//!
//! `peer_credentials_for_stream` reads the kernel-recorded
//! `(pid, uid)` of the connected peer via SO_PEERCRED on Linux
//! and `getpeereid` + `LOCAL_PEERPID` on macOS. The serve side
//! uses the result to gate every accepted connection through
//! [`authorize_server_peer`] (only the same uid as the serve
//! process is allowed). The client side uses
//! [`authorize_client_peer`] to verify after `WhoAmI` that the
//! socket really belongs to the session whose path it
//! advertises and that the peer pid matches the recorded owner
//! pid.
//!
//! `session_id_from_ipc_path` turns the on-disk socket file
//! name (`<session-id>.sock`) into the session id the path
//! advertises; it is the authoritative source for the
//! "expected session id" on the client side.
//!
//! [`publish_ipc_socket`] places the per-session socket on
//! disk under the platform's runtime root, locks down its
//! directory and file modes (700/600), and stamps the path
//! into `serve_sessions`. [`cleanup_published_ipc_socket`]
//! reverses that on shutdown or failure. The supporting
//! helpers (`ipc_socket_location`, `ensure_secure_ipc_directory`,
//! `clear_stale_ipc_socket`, `listen_with_backlog`,
//! `audit_bound_ipc_socket`) are private to this file.

#![cfg(unix)]

use std::fs;
use std::io;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::mem::size_of;
#[cfg(target_os = "linux")]
use std::mem::zeroed;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
#[cfg(target_os = "linux")]
use std::path::PathBuf;

use rusqlite::{params, Connection};

use super::{IpcError, IpcPeerCredentials, IpcSocketLocation, PublishedIpcSocket};
use crate::core::vault_sync::{current_effective_uid, VaultSyncError};

pub(crate) fn session_id_from_ipc_path(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(ToOwned::to_owned)
}

pub(crate) fn peer_credentials_for_stream(
    stream: &UnixStream,
) -> Result<IpcPeerCredentials, VaultSyncError> {
    let fd = stream.as_raw_fd();
    #[cfg(target_os = "linux")]
    {
        #[expect(
            unsafe_code,
            reason = "MaybeUninit-style zero-init of libc::ucred which is plain-old-data; subsequent getsockopt fills it in"
        )]
        let mut creds: libc::ucred = unsafe { zeroed() };
        let mut len = size_of::<libc::ucred>() as libc::socklen_t;
        #[expect(
            unsafe_code,
            reason = "POSIX getsockopt SO_PEERCRED is a syscall; we pass a valid fd, a stable libc::ucred buffer, and a matching length"
        )]
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut creds as *mut libc::ucred).cast(),
                &mut len,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(IpcPeerCredentials {
            pid: creds.pid,
            uid: creds.uid,
        })
    }
    #[cfg(target_os = "macos")]
    {
        let mut uid: libc::uid_t = 0;
        let mut gid: libc::gid_t = 0;
        #[expect(
            unsafe_code,
            reason = "POSIX getpeereid syscall; we pass a valid fd and stack-allocated outputs"
        )]
        let rc = unsafe { libc::getpeereid(fd, &mut uid, &mut gid) };
        if rc != 0 {
            return Err(io::Error::last_os_error().into());
        }
        let mut pid: libc::pid_t = 0;
        let mut len = size_of::<libc::pid_t>() as libc::socklen_t;
        #[expect(
            unsafe_code,
            reason = "macOS LOCAL_PEERPID getsockopt syscall; we pass a valid fd, a stable pid_t buffer, and a matching length"
        )]
        let rc = unsafe {
            libc::getsockopt(
                fd,
                0,
                libc::LOCAL_PEERPID,
                (&mut pid as *mut libc::pid_t).cast(),
                &mut len,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(IpcPeerCredentials {
            pid,
            uid: uid as u32,
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err(VaultSyncError::InvariantViolation {
            message: "peer credentials unsupported on this unix platform".to_owned(),
        })
    }
}

pub(crate) fn authorize_server_peer(
    socket_path: &Path,
    peer: &IpcPeerCredentials,
) -> Result<(), VaultSyncError> {
    if peer.uid != current_effective_uid() {
        return Err(VaultSyncError::Ipc(IpcError::IpcPeerAuthFailed {
            path: socket_path.display().to_string(),
            reason: format!(
                "peer uid {} does not match current uid {}",
                peer.uid,
                current_effective_uid()
            ),
        }));
    }
    Ok(())
}

pub(crate) fn authorize_client_peer(
    socket_path: &Path,
    path_session_id: &str,
    owner_session_id: &str,
    owner_pid: i64,
    peer: &IpcPeerCredentials,
    whoami_session_id: &str,
) -> Result<(), VaultSyncError> {
    if path_session_id != owner_session_id {
        return Err(VaultSyncError::Ipc(IpcError::IpcPeerAuthFailed {
            path: socket_path.display().to_string(),
            reason: format!(
                "path session {} does not match owner session {}",
                path_session_id, owner_session_id
            ),
        }));
    }
    if peer.uid != current_effective_uid() {
        return Err(VaultSyncError::Ipc(IpcError::IpcPeerAuthFailed {
            path: socket_path.display().to_string(),
            reason: format!(
                "peer uid {} does not match current uid {}",
                peer.uid,
                current_effective_uid()
            ),
        }));
    }
    if i64::from(peer.pid) != owner_pid {
        return Err(VaultSyncError::Ipc(IpcError::IpcPeerAuthFailed {
            path: socket_path.display().to_string(),
            reason: format!(
                "peer pid {} does not match owner pid {}",
                peer.pid, owner_pid
            ),
        }));
    }
    if whoami_session_id != path_session_id {
        return Err(VaultSyncError::Ipc(IpcError::IpcPeerAuthFailed {
            path: socket_path.display().to_string(),
            reason: format!(
                "whoami session {} does not match path session {}",
                whoami_session_id, path_session_id
            ),
        }));
    }
    Ok(())
}

pub(in crate::core::vault_sync) fn publish_ipc_socket(
    conn: &Connection,
    session_id: &str,
) -> Result<PublishedIpcSocket, VaultSyncError> {
    let location = ipc_socket_location()?;
    ensure_secure_ipc_directory(&location.runtime_root, location.create_runtime_root)?;
    ensure_secure_ipc_directory(&location.socket_dir, true)?;
    let socket_path = location.socket_dir.join(format!("{session_id}.sock"));
    if socket_path.exists() {
        clear_stale_ipc_socket(&socket_path)?;
    }
    let listener = UnixListener::bind(&socket_path)?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    listen_with_backlog(&listener)?;
    audit_bound_ipc_socket(&socket_path)?;
    conn.execute(
        "UPDATE serve_sessions SET ipc_path = ?1 WHERE session_id = ?2",
        params![socket_path.display().to_string(), session_id],
    )?;
    Ok(PublishedIpcSocket {
        listener,
        path: socket_path,
    })
}

pub(in crate::core::vault_sync) fn cleanup_published_ipc_socket(
    conn: &Connection,
    session_id: &str,
    socket_path: &Path,
) -> Result<(), VaultSyncError> {
    if socket_path.exists() {
        let _ = fs::remove_file(socket_path);
    }
    conn.execute(
        "UPDATE serve_sessions SET ipc_path = NULL WHERE session_id = ?1",
        [session_id],
    )?;
    Ok(())
}

fn ipc_socket_location() -> Result<IpcSocketLocation, VaultSyncError> {
    #[cfg(target_os = "linux")]
    {
        if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
            let runtime_root = PathBuf::from(runtime_dir);
            return Ok(IpcSocketLocation {
                socket_dir: runtime_root.join("quaid"),
                runtime_root,
                create_runtime_root: false,
            });
        }
        dirs::home_dir()
            .map(|home| {
                let runtime_root = home.join(".cache").join("quaid");
                IpcSocketLocation {
                    socket_dir: runtime_root.join("run"),
                    runtime_root,
                    create_runtime_root: true,
                }
            })
            .ok_or_else(|| VaultSyncError::InvariantViolation {
                message: "unable to resolve HOME for IPC directory".to_owned(),
            })
    }
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir()
            .map(|home| {
                let runtime_root = home
                    .join("Library")
                    .join("Application Support")
                    .join("quaid");
                IpcSocketLocation {
                    socket_dir: runtime_root.join("run"),
                    runtime_root,
                    create_runtime_root: true,
                }
            })
            .ok_or_else(|| VaultSyncError::InvariantViolation {
                message: "unable to resolve HOME for IPC directory".to_owned(),
            })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        dirs::home_dir()
            .map(|home| {
                let runtime_root = home.join(".cache").join("quaid");
                IpcSocketLocation {
                    socket_dir: runtime_root.join("run"),
                    runtime_root,
                    create_runtime_root: true,
                }
            })
            .ok_or_else(|| VaultSyncError::InvariantViolation {
                message: "unable to resolve HOME for IPC directory".to_owned(),
            })
    }
}

fn ensure_secure_ipc_directory(path: &Path, create_if_missing: bool) -> Result<(), VaultSyncError> {
    if !path.exists() {
        if !create_if_missing {
            return Err(VaultSyncError::Ipc(IpcError::IpcDirectoryInsecure {
                path: path.display().to_string(),
                reason: "path does not exist".to_owned(),
            }));
        }
        fs::create_dir_all(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    let metadata = fs::symlink_metadata(path)?;
    let mode = metadata.mode() & 0o777;
    if !metadata.file_type().is_dir() {
        return Err(VaultSyncError::Ipc(IpcError::IpcDirectoryInsecure {
            path: path.display().to_string(),
            reason: "path is not a directory".to_owned(),
        }));
    }
    if metadata.uid() != current_effective_uid() {
        return Err(VaultSyncError::Ipc(IpcError::IpcDirectoryInsecure {
            path: path.display().to_string(),
            reason: format!(
                "owner uid {} does not match current uid {}",
                metadata.uid(),
                current_effective_uid()
            ),
        }));
    }
    if mode != 0o700 {
        return Err(VaultSyncError::Ipc(IpcError::IpcDirectoryInsecure {
            path: path.display().to_string(),
            reason: format!("mode {:o} is not 700", mode),
        }));
    }
    Ok(())
}

fn clear_stale_ipc_socket(path: &Path) -> Result<(), VaultSyncError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_socket() {
        return Err(VaultSyncError::Ipc(IpcError::IpcSocketCollision {
            path: path.display().to_string(),
            reason: "existing path is not a unix socket".to_owned(),
        }));
    }
    match UnixStream::connect(path) {
        Ok(stream) => {
            let creds = peer_credentials_for_stream(&stream)?;
            return Err(VaultSyncError::Ipc(IpcError::IpcSocketCollision {
                path: path.display().to_string(),
                reason: format!("live listener already bound by pid {}", creds.pid),
            }));
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused
                    | io::ErrorKind::NotFound
                    | io::ErrorKind::TimedOut
                    | io::ErrorKind::ConnectionAborted
            ) =>
        {
            fs::remove_file(path)?;
        }
        Err(error) => {
            return Err(VaultSyncError::Ipc(IpcError::IpcSocketCollision {
                path: path.display().to_string(),
                reason: error.to_string(),
            }));
        }
    }
    Ok(())
}

fn listen_with_backlog(listener: &UnixListener) -> Result<(), VaultSyncError> {
    #[expect(
        unsafe_code,
        reason = "POSIX listen() is a syscall; we pass a valid file descriptor obtained from UnixListener::as_raw_fd"
    )]
    let rc = unsafe { libc::listen(listener.as_raw_fd(), 16) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error().into())
    }
}

pub(in crate::core::vault_sync) fn audit_bound_ipc_socket(
    path: &Path,
) -> Result<(), VaultSyncError> {
    let metadata = fs::symlink_metadata(path)?;
    let mode = metadata.mode() & 0o777;
    if !metadata.file_type().is_socket() {
        return Err(VaultSyncError::Ipc(IpcError::IpcSocketPermission {
            path: path.display().to_string(),
            reason: "bound path is not a unix socket".to_owned(),
        }));
    }
    if metadata.uid() != current_effective_uid() {
        return Err(VaultSyncError::Ipc(IpcError::IpcSocketPermission {
            path: path.display().to_string(),
            reason: format!(
                "owner uid {} does not match current uid {}",
                metadata.uid(),
                current_effective_uid()
            ),
        }));
    }
    if mode != 0o600 {
        return Err(VaultSyncError::Ipc(IpcError::IpcSocketPermission {
            path: path.display().to_string(),
            reason: format!("mode {:o} is not 600", mode),
        }));
    }
    Ok(())
}
