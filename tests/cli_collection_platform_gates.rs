#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests asserting that the vault-sync CLI surfaces (`add`,
//! `sync`, `restore`, `audit`) refuse to run on non-Unix platforms with the
//! `UnsupportedPlatformError` payload.

#[cfg(not(unix))]
use quaid::commands::collection::{
    run, CollectionAction, CollectionAddArgs, CollectionAuditArgs, CollectionRestoreArgs,
    CollectionSyncArgs,
};

#[path = "common/collection_fixtures.rs"]
mod fixtures;
#[cfg(not(unix))]
use fixtures::open_test_db;

#[cfg(not(unix))]
#[test]
fn add_refuses_windows_platform() {
    let conn = open_test_db();
    let root = tempfile::TempDir::new().unwrap();

    let error = run(
        &conn,
        CollectionAction::Add(CollectionAddArgs {
            name: "work".to_owned(),
            path: root.path().to_path_buf(),
            read_only: false,
            writable: false,
            write_quaid_id: false,
            namespace: None,
        }),
        true,
    )
    .unwrap_err();

    assert!(error.to_string().contains("UnsupportedPlatformError"));
}

#[cfg(not(unix))]
#[test]
fn sync_refuses_windows_platform() {
    let conn = open_test_db();

    let error = run(
        &conn,
        CollectionAction::Sync(CollectionSyncArgs {
            name: "work".to_owned(),
            remap_root: None,
            finalize_pending: false,
            online: false,
            no_embed: false,
        }),
        true,
    )
    .unwrap_err();

    assert!(error.to_string().contains("UnsupportedPlatformError"));
}

#[cfg(not(unix))]
#[test]
fn restore_refuses_windows_platform() {
    let conn = open_test_db();
    let target = tempfile::TempDir::new().unwrap();

    let error = run(
        &conn,
        CollectionAction::Restore(CollectionRestoreArgs {
            name: "work".to_owned(),
            target: target.path().to_path_buf(),
            online: false,
        }),
        true,
    )
    .unwrap_err();

    assert!(error.to_string().contains("UnsupportedPlatformError"));
}

#[cfg(not(unix))]
#[test]
fn audit_refuses_windows_platform() {
    let conn = open_test_db();

    let error = run(
        &conn,
        CollectionAction::Audit(CollectionAuditArgs {
            name: "work".to_owned(),
            raw_imports_gc: false,
        }),
        true,
    )
    .unwrap_err();

    assert!(error.to_string().contains("UnsupportedPlatformError"));
}
