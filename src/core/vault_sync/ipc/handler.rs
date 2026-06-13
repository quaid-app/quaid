//! Per-stream IPC handler and accept loop.
//!
//! [`accept_ipc_clients`] is the non-blocking accept loop the
//! supervisor calls each tick: it drains pending connections,
//! enforces [`IPC_HANDLER_LIMIT`] via an `AtomicUsize` counter,
//! and offloads each accepted stream to a dedicated thread so
//! a slow or chatty client can't stall the supervisor's
//! heartbeat / watcher poll cadence.
//!
//! Per-thread, [`handle_ipc_client`] runs the request loop:
//! authenticates the peer via SO_PEERCRED / `getpeereid`
//! through [`super::socket::authorize_server_peer`], then
//! reads JSON-line `IpcRequest` envelopes off the stream and
//! dispatches `WhoAmI` / `Put` against the same SQLite
//! database the serve process owns.
//!
//! [`IpcHandlerGuard`] is a Drop-on-thread-exit decrement so
//! the in-flight counter is released even on early-return or
//! panic, and [`write_ipc_response`] is the single egress
//! point that serialises and flushes one response.

#![cfg(unix)]

use std::io;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use rusqlite::Connection;

use super::socket::{authorize_server_peer, peer_credentials_for_stream};
use super::{IpcRequest, IpcResponse};
// `put` still lives in the CLI layer; relocating the write path into core is the
// deferred `core::pages::write_page` unification (review §16, steps 2/3) and
// overlaps with the unmerged write-path stack.
use crate::commands::put;
use crate::core::vault_sync::VaultSyncError;

/// Maximum concurrent in-flight IPC handler threads.  Connections that arrive
/// when this cap is reached are immediately closed so a rogue same-UID caller
/// cannot exhaust OS thread resources and impact serve liveness.
pub(in crate::core::vault_sync) const IPC_HANDLER_LIMIT: usize = 8;

/// RAII guard: decrements the in-flight counter when dropped so the slot is
/// always released even if the handler returns early or panics.
pub(in crate::core::vault_sync) struct IpcHandlerGuard(
    pub(in crate::core::vault_sync) Arc<AtomicUsize>,
);

impl Drop for IpcHandlerGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(in crate::core::vault_sync) fn accept_ipc_clients(
    listener: &UnixListener,
    socket_path: &Path,
    db_path: &str,
    session_id: &str,
    in_flight: &Arc<AtomicUsize>,
) {
    loop {
        match listener.accept() {
            Ok((stream, _addr)) => {
                if in_flight.fetch_add(1, Ordering::AcqRel) >= IPC_HANDLER_LIMIT {
                    in_flight.fetch_sub(1, Ordering::AcqRel);
                    eprintln!(
                        "WARN: ipc_handler_limit_reached path={} limit={} connection_closed",
                        socket_path.display(),
                        IPC_HANDLER_LIMIT,
                    );
                    drop(stream);
                    break;
                }
                let guard = IpcHandlerGuard(Arc::clone(in_flight));
                let socket_path_owned = socket_path.to_path_buf();
                let db_path_owned = db_path.to_owned();
                let session_id_owned = session_id.to_owned();
                thread::spawn(move || {
                    let _guard = guard;
                    if let Err(error) = handle_ipc_client(
                        stream,
                        &socket_path_owned,
                        &db_path_owned,
                        &session_id_owned,
                    ) {
                        eprintln!(
                            "WARN: ipc_client_failed path={} error={}",
                            socket_path_owned.display(),
                            error
                        );
                    }
                });
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) => {
                eprintln!(
                    "WARN: ipc_accept_failed path={} error={}",
                    socket_path.display(),
                    error
                );
                break;
            }
        }
    }
}

fn handle_ipc_client(
    stream: UnixStream,
    socket_path: &Path,
    db_path: &str,
    session_id: &str,
) -> Result<(), VaultSyncError> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let peer = peer_credentials_for_stream(&stream)?;
    authorize_server_peer(socket_path, &peer)?;
    eprintln!(
        "INFO: ipc_peer_authenticated session_id={} peer_pid={} peer_uid={}",
        session_id, peer.pid, peer.uid
    );

    let read_stream = stream.try_clone()?;
    let mut reader = BufReader::new(read_stream);
    let mut writer = BufWriter::new(stream);
    loop {
        let mut line = String::new();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            break;
        }
        let request = match serde_json::from_str::<IpcRequest>(line.trim_end()) {
            Ok(request) => request,
            Err(error) => {
                write_ipc_response(
                    &mut writer,
                    &IpcResponse::Error {
                        error: format!("invalid ipc request: {error}"),
                    },
                )?;
                break;
            }
        };
        match request {
            IpcRequest::WhoAmI => {
                write_ipc_response(
                    &mut writer,
                    &IpcResponse::WhoAmI {
                        session_id: session_id.to_owned(),
                    },
                )?;
            }
            IpcRequest::Put {
                slug,
                content,
                expected_version,
            } => {
                let conn = Connection::open(db_path)?;
                match put::put_from_string_status(&conn, &slug, &content, expected_version) {
                    Ok(status) => {
                        write_ipc_response(&mut writer, &IpcResponse::PutOk { status })?;
                    }
                    Err(error) => {
                        write_ipc_response(
                            &mut writer,
                            &IpcResponse::Error {
                                error: error.to_string(),
                            },
                        )?;
                    }
                }
                break;
            }
        }
    }
    Ok(())
}

fn write_ipc_response(
    writer: &mut BufWriter<UnixStream>,
    response: &IpcResponse,
) -> Result<(), VaultSyncError> {
    serde_json::to_writer(&mut *writer, response).map_err(|error| {
        VaultSyncError::InvariantViolation {
            message: format!("failed to serialize ipc response: {error}"),
        }
    })?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}
