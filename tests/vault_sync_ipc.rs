#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::too_many_lines,
    unused_imports,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites; unused_imports because the broad import header is shared across all vault_sync_*.rs files but each only consumes a subset"
)]

//! IPC socket lifecycle and peer auth tests.
//!
//! Migrated verbatim from `src/core/vault_sync.rs::tests` (the pre-extraction
//! inline `mod tests` block). Test bodies are unchanged; only `use` paths were
//! rewritten to the public crate path. White-box tests that touch private
//! items remain inline in `src/core/vault_sync.rs`.

#[path = "common/vault_sync_fixtures.rs"]
mod fixtures;

use fixtures::*;

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use quaid::core::collections::{Collection, CollectionState};
use quaid::core::db;
#[cfg(unix)]
use quaid::core::file_state;
use quaid::core::fs_safety;
use quaid::core::markdown;
use quaid::core::raw_imports;
use quaid::core::vault_sync::*;

#[cfg(all(unix, target_os = "linux"))]
#[test]
fn start_serve_runtime_refuses_insecure_ipc_directory_permissions() {
    let _env_lock = env_mutation_lock().lock().unwrap();
    let runtime_root = secure_runtime_root();
    let socket_dir = runtime_root.path().join("quaid");
    fs::create_dir_all(&socket_dir).unwrap();
    fs::set_permissions(&socket_dir, fs::Permissions::from_mode(0o755)).unwrap();
    let _xdg = EnvVarGuard::set("XDG_RUNTIME_DIR", runtime_root.path().to_str().unwrap());
    let (_dir, db_path, _conn) = open_test_db_file();

    let error = match start_serve_runtime(db_path) {
        Ok(_) => panic!("expected insecure socket directory to refuse startup"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        VaultSyncError::Ipc(IpcError::IpcDirectoryInsecure { .. })
    ));
    assert!(error.to_string().contains("IpcDirectoryInsecureError"));
}

#[cfg(all(unix, target_os = "linux"))]
#[test]
fn start_serve_runtime_refuses_insecure_xdg_runtime_root_permissions() {
    let _env_lock = env_mutation_lock().lock().unwrap();
    let runtime_root = tempfile::TempDir::new().unwrap();
    fs::set_permissions(runtime_root.path(), fs::Permissions::from_mode(0o755)).unwrap();
    let _xdg = EnvVarGuard::set("XDG_RUNTIME_DIR", runtime_root.path().to_str().unwrap());
    let (_dir, db_path, _conn) = open_test_db_file();

    let error = match start_serve_runtime(db_path) {
        Ok(_) => panic!("expected insecure XDG runtime root to refuse startup"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        VaultSyncError::Ipc(IpcError::IpcDirectoryInsecure { .. })
    ));
    assert!(error.to_string().contains("IpcDirectoryInsecureError"));
    assert!(error.to_string().contains("mode 755 is not 700"));
}

#[cfg(all(unix, target_os = "linux"))]
#[test]
fn start_serve_runtime_refuses_insecure_fallback_runtime_root_permissions() {
    let _env_lock = env_mutation_lock().lock().unwrap();
    let home = tempfile::TempDir::new().unwrap();
    let runtime_root = home.path().join(".cache").join("quaid");
    fs::create_dir_all(&runtime_root).unwrap();
    fs::set_permissions(&runtime_root, fs::Permissions::from_mode(0o755)).unwrap();
    let _xdg = EnvVarGuard::clear("XDG_RUNTIME_DIR");
    let _home = EnvVarGuard::set("HOME", home.path().to_str().unwrap());
    let (_dir, db_path, _conn) = open_test_db_file();

    let error = match start_serve_runtime(db_path) {
        Ok(_) => panic!("expected insecure fallback runtime root to refuse startup"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        VaultSyncError::Ipc(IpcError::IpcDirectoryInsecure { .. })
    ));
    assert!(error.to_string().contains("IpcDirectoryInsecureError"));
    assert!(error.to_string().contains(runtime_root.to_str().unwrap()));
}

#[test]
fn serve_ipc_source_publishes_after_audit_and_cleans_up_before_unregister() {
    let socket_source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("core")
            .join("vault_sync")
            .join("ipc")
            .join("socket.rs"),
    )
    .unwrap();
    let publish_start = socket_source
        .find("fn publish_ipc_socket(")
        .expect("publish helper present");
    let publish_end = socket_source[publish_start..]
        .find("fn cleanup_published_ipc_socket(")
        .map(|offset| publish_start + offset)
        .expect("publish helper boundary");
    let publish_source = &socket_source[publish_start..publish_end];
    let runtime_root_idx = publish_source
        .find("ensure_secure_ipc_directory(&location.runtime_root")
        .expect("secure runtime root check");
    let dir_idx = publish_source
        .find("ensure_secure_ipc_directory(&location.socket_dir, true)")
        .expect("secure socket directory check");
    let stale_idx = publish_source
        .find("clear_stale_ipc_socket(&socket_path)")
        .expect("stale socket cleanup");
    let set_perms_idx = publish_source
        .find("fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))")
        .expect("explicit socket permissions set after bind");
    let audit_idx = publish_source
        .find("audit_bound_ipc_socket(&socket_path)")
        .expect("bind-time audit");
    let publish_idx = publish_source
        .find("UPDATE serve_sessions SET ipc_path = ?1 WHERE session_id = ?2")
        .expect("ipc path publish");
    assert!(
        runtime_root_idx < dir_idx
            && dir_idx < stale_idx
            && stale_idx < set_perms_idx
            && set_perms_idx < audit_idx
            && audit_idx < publish_idx
    );

    let mod_source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("core")
            .join("vault_sync")
            .join("mod.rs"),
    )
    .unwrap();
    let runtime_start = mod_source
        .find("pub fn start_serve_runtime(")
        .expect("serve runtime present");
    let runtime_end = mod_source[runtime_start..]
        .find("\npub fn remap_collection(")
        .map(|offset| runtime_start + offset)
        .expect("serve runtime boundary (next pub fn after run_supervisor_loop)");
    let runtime_source = &mod_source[runtime_start..runtime_end];
    let cleanup_idx = runtime_source
        .find("cleanup_published_ipc_socket(&conn, &session_id_for_thread, &published_ipc.path)")
        .expect("ipc cleanup call");
    let unregister_idx = runtime_source
        .find("unregister_session(&conn, &session_id_for_thread)")
        .expect("session unregister call");
    assert!(cleanup_idx < unregister_idx);
}

#[test]
fn serve_ipc_source_refuses_cross_uid_peer_before_request_dispatch() {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("core")
            .join("vault_sync")
            .join("ipc")
            .join("handler.rs"),
    )
    .unwrap();
    let handler_start = source
        .find("fn handle_ipc_client(")
        .expect("ipc handler present");
    let handler_end = source[handler_start..]
        .find("fn write_ipc_response(")
        .map(|offset| handler_start + offset)
        .expect("ipc handler boundary");
    let handler_source = &source[handler_start..handler_end];
    let peer_idx = handler_source
        .find("let peer = peer_credentials_for_stream(&stream)?;")
        .expect("peer credential lookup");
    let auth_idx = handler_source
        .find("authorize_server_peer(socket_path, &peer)?;")
        .expect("server peer auth");
    let parse_idx = handler_source
        .find("serde_json::from_str::<IpcRequest>(line.trim_end())")
        .expect("request parse");
    assert!(peer_idx < auth_idx && auth_idx < parse_idx);
}

/// Regression: `accept_ipc_clients` must offload each accepted connection to a
/// dedicated thread via `thread::spawn` so that blocking IPC I/O (up to 5 s per
/// read/write timeout) cannot stall the main serve loop and trigger a false-dead
/// live-owner verdict.  We verify the structural invariant via source inspection.
#[test]
fn accept_ipc_clients_offloads_each_client_to_its_own_thread() {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("core")
            .join("vault_sync")
            .join("ipc")
            .join("handler.rs"),
    )
    .unwrap();
    let fn_start = source
        .find("fn accept_ipc_clients(")
        .expect("accept_ipc_clients fn present");
    // Boundary: the next fn after accept_ipc_clients is handle_ipc_client.
    let fn_end = source[fn_start..]
        .find("fn handle_ipc_client(")
        .map(|offset| fn_start + offset)
        .expect("handle_ipc_client fn follows accept_ipc_clients");
    let fn_body = &source[fn_start..fn_end];

    // The function must not call handle_ipc_client directly on the main loop thread.
    assert!(
        !fn_body.contains("handle_ipc_client(stream,"),
        "accept_ipc_clients must not call handle_ipc_client inline on the main thread"
    );
    // The function must use thread::spawn to offload.
    assert!(
        fn_body.contains("thread::spawn("),
        "accept_ipc_clients must offload each client via thread::spawn"
    );
    // The spawn closure must contain the handle_ipc_client call.
    let spawn_idx = fn_body.find("thread::spawn(").unwrap();
    let spawn_region = &fn_body[spawn_idx..];
    assert!(
        spawn_region.contains("handle_ipc_client("),
        "handle_ipc_client must be called inside the thread::spawn closure"
    );
    // The function must enforce the in-flight cap before spawning.
    assert!(
        fn_body.contains("IPC_HANDLER_LIMIT"),
        "accept_ipc_clients must reference IPC_HANDLER_LIMIT"
    );
    assert!(
        fn_body.contains("fetch_add("),
        "accept_ipc_clients must use fetch_add for the in-flight counter"
    );
    // Rollback path: fetch_sub must appear for the saturation branch.
    assert!(
        fn_body.contains("fetch_sub("),
        "accept_ipc_clients must roll back via fetch_sub when saturated"
    );
    // Guard must be created before the spawn closure.
    assert!(
        fn_body.contains("IpcHandlerGuard("),
        "accept_ipc_clients must construct an IpcHandlerGuard before spawning"
    );
}
