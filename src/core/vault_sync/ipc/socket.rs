//! Peer-credential and per-session authorisation for the IPC
//! socket (`cfg(unix)` only).
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

#![cfg(unix)]

use std::io;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::mem::size_of;
#[cfg(target_os = "linux")]
use std::mem::zeroed;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::Path;

use super::{IpcError, IpcPeerCredentials};
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
