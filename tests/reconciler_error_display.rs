#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! ReconcileError Display impls and CollectionDirtyStatus is_dirty checks.

#[path = "common/reconciler_fixtures.rs"]
mod common_reconciler_fixtures;

use common_reconciler_fixtures::*;
use quaid::core::reconciler::*;
use tempfile::TempDir;

#[test]
fn reconcile_error_display_collection_lacks_writer_quiescence() {
    let err = ReconcileError::CollectionLacksWriterQuiescenceError {
        collection_name: "my-vault".to_owned(),
        root_path: "/mnt/vault".to_owned(),
    };
    let s = err.to_string();
    assert!(s.contains("CollectionLacksWriterQuiescenceError"));
    assert!(s.contains("my-vault"));
    assert!(s.contains("/mnt/vault"));
}

#[test]
fn reconcile_error_display_collection_dirty_error() {
    let err = ReconcileError::CollectionDirtyError {
        collection_name: "dirty-vault".to_owned(),
        status: CollectionDirtyStatus {
            needs_full_sync: true,
            sentinel_count: 2,
            recovery_in_progress: false,
            last_sync_at: Some("2024-01-01T00:00:00Z".to_owned()),
        },
    };
    let s = err.to_string();
    assert!(s.contains("CollectionDirtyError"));
    assert!(s.contains("dirty-vault"));
    assert!(s.contains("needs_full_sync=true"));
    assert!(s.contains("sentinel_count=2"));
}

#[test]
fn reconcile_error_display_remap_drift_conflict_error() {
    let err = ReconcileError::RemapDriftConflictError {
        collection_name: "remap-vault".to_owned(),
        summary: DriftCaptureSummary {
            pages_updated: 1,
            pages_added: 2,
            pages_quarantined: 0,
            pages_deleted: 0,
        },
    };
    let s = err.to_string();
    assert!(s.contains("RemapDriftConflictError"));
    assert!(s.contains("remap-vault"));
    assert!(s.contains("pages_updated=1"));
}

#[test]
fn reconcile_error_display_collection_unstable_error() {
    let err = ReconcileError::CollectionUnstableError {
        collection_name: "unstable-vault".to_owned(),
        operation: RestoreRemapOperation::Remap,
        phase: "stability",
        retries: 5,
    };
    let s = err.to_string();
    assert!(s.contains("CollectionUnstableError"));
    assert!(s.contains("unstable-vault"));
    assert!(s.contains("operation=remap"));
    assert!(s.contains("retries=5"));
}

#[test]
fn collection_dirty_status_is_clean_without_flags_or_sentinels() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let recovery_root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());

    let status = is_collection_dirty(&conn, collection.id, recovery_root.path()).unwrap();

    assert!(!status.is_dirty());
    assert!(!status.needs_full_sync);
    assert_eq!(status.sentinel_count, 0);
    assert!(!status.recovery_in_progress);
    assert!(status.last_sync_at.is_none());
}

#[test]
fn collection_dirty_status_is_dirty_when_only_sentinel_count_nonzero() {
    let status = CollectionDirtyStatus {
        needs_full_sync: false,
        sentinel_count: 1,
        recovery_in_progress: false,
        last_sync_at: None,
    };
    assert!(status.is_dirty());
}

#[test]
fn collection_dirty_status_is_not_dirty_when_all_clear() {
    let status = CollectionDirtyStatus {
        needs_full_sync: false,
        sentinel_count: 0,
        recovery_in_progress: false,
        last_sync_at: None,
    };
    assert!(!status.is_dirty());
}
