#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::too_many_lines,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! `remap_collection` / `verify_remap_root` and tree-fence source tests.
//!
//! Migrated verbatim from `src/core/vault_sync.rs::tests` (the pre-extraction
//! inline `mod tests` block). Test bodies are unchanged; only `use` paths were
//! rewritten to the public crate path. White-box tests that touch private
//! items remain inline in `src/core/vault_sync.rs`.

#[path = "common/vault_sync_fixtures.rs"]
mod fixtures;

use fixtures::*;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use quaid::core::collections::{Collection, CollectionState};
use quaid::core::db;
use quaid::core::fs_safety;
use quaid::core::markdown;
use quaid::core::raw_imports;
#[cfg(unix)]
use quaid::core::file_state;
use quaid::core::vault_sync::*;

#[test]
fn verify_remap_root_rejects_invalid_quaidignore_in_new_root() {
    let conn = open_test_db();
    let old_root = tempfile::TempDir::new().unwrap();
    let new_root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", old_root.path());
    let raw_bytes =
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "11111111-1111-7111-8111-111111111111",
        "hello world from note a",
        raw_bytes,
        "notes/a.md",
    );
    fs::create_dir_all(new_root.path().join("notes")).unwrap();
    fs::write(new_root.path().join("notes").join("a.md"), raw_bytes).unwrap();
    fs::write(new_root.path().join(".quaidignore"), "[broken\n").unwrap();

    let collection = load_collection_by_id(&conn, collection_id).unwrap();
    let error = verify_remap_root(&conn, &collection, new_root.path()).unwrap_err();

    assert!(matches!(error, VaultSyncError::InvariantViolation { .. }));
    assert!(error
        .to_string()
        .contains("invalid .quaidignore in remap root"));
}

#[cfg(unix)]
#[test]
fn remap_online_source_defers_attach_to_rcrt_and_remap_attach_reason_appears_once() {
    let source = production_vault_sync_source();

    let remap_start = source.find("pub fn remap_collection(").unwrap();
    let remap_end = source[remap_start..]
        .find("pub fn verify_remap_root(")
        .map(|offset| remap_start + offset)
        .unwrap();
    let remap_source = &source[remap_start..remap_end];
    assert!(
        remap_source.contains(
            "wait_for_exact_ack(conn, collection.id, &expected_session_id, generation)?"
        ),
        "online remap must wait for the exact watcher ack before mutating DB state"
    );
    assert!(
        remap_source.contains("needs_full_sync = 1"),
        "online remap must arm the write gate for the post-remap attach pass"
    );
    assert!(
        remap_source.contains("DELETE FROM file_state WHERE collection_id = ?1"),
        "online remap must limit itself to the DB state flip plus file_state reset"
    );
    let online_source = &remap_source[..remap_source.find("} else {").unwrap()];
    assert!(
        !online_source.contains("complete_attach(")
            && !online_source.contains("full_hash_reconcile_authorized("),
        "remap_collection must not run attach or full-hash reconcile inline"
    );

    let rcrt_start = source.find("pub fn run_rcrt_pass(").unwrap();
    let rcrt_end = source[rcrt_start..]
        .find("fn embedding_drain_interval_secs(")
        .map(|offset| rcrt_start + offset)
        .unwrap();
    let rcrt_source = &source[rcrt_start..rcrt_end];
    assert_eq!(
        rcrt_source
            .matches("AttachReason::RemapPostReconcile")
            .count(),
        1,
        "RCRT should have exactly one remap attach arm"
    );
    assert!(
        rcrt_source.contains("if complete_attach(conn, collection_id, session_id, reason)? {"),
        "RCRT must own the attach transition after remap"
    );
}

#[cfg(unix)]
#[test]
fn offline_remap_runs_reconcile_inline_and_preserves_uuid_identity_across_reorganization() {
    let (_db_dir, _db_path, conn) = open_test_db_file();
    let old_root = tempfile::TempDir::new().unwrap();
    let new_root = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(old_root.path().join("notes")).unwrap();
    let collection_id = insert_collection(&conn, "work", old_root.path());
    let raw_bytes_a =
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\nslug: notes/a\n---\nhello world from note a";
    let raw_bytes_b =
        b"---\nmemory_id: 22222222-2222-7222-8222-222222222222\nslug: notes/b\n---\nhello world from note b";
    let page_a = insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "11111111-1111-7111-8111-111111111111",
        "hello world from note a",
        raw_bytes_a,
        "notes/old-a.md",
    );
    let page_b = insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/b",
        "22222222-2222-7222-8222-222222222222",
        "hello world from note b",
        raw_bytes_b,
        "notes/b.md",
    );
    fs::write(old_root.path().join("notes").join("old-a.md"), raw_bytes_a).unwrap();
    fs::write(old_root.path().join("notes").join("b.md"), raw_bytes_b).unwrap();
    fs::create_dir_all(new_root.path().join("notes")).unwrap();
    fs::create_dir_all(new_root.path().join("nested")).unwrap();
    fs::write(
        new_root.path().join("nested").join("renamed-a.md"),
        raw_bytes_a,
    )
    .unwrap();
    fs::write(new_root.path().join("notes").join("b.md"), raw_bytes_b).unwrap();
    conn.execute(
        "INSERT INTO links (from_page_id, to_page_id, relationship, context, source_kind)
         VALUES (?1, ?2, 'depends_on', '', 'programmatic')",
        params![page_a, page_b],
    )
    .unwrap();

    let summary = remap_collection(&conn, "work", new_root.path(), false).unwrap();

    let collection = load_collection_by_id(&conn, collection_id).unwrap();
    assert_eq!(collection.root_path, new_root.path().display().to_string());
    assert_eq!(collection.state, CollectionState::Active);
    assert!(!collection.needs_full_sync);
    assert_eq!(summary.resolved_pages, 2);
    assert!(collection.active_lease_session_id.is_none());
    assert!(collection.restore_lease_session_id.is_none());
    assert!(owner_session_id(&conn, collection_id).unwrap().is_none());
    ensure_collection_write_allowed(&conn, collection_id).unwrap();
    let remapped_page_id: i64 = conn
        .query_row(
            "SELECT id FROM pages WHERE collection_id = ?1 AND slug = 'notes/a'",
            [collection_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remapped_page_id, page_a);
    let relative_path: String = conn
        .query_row(
            "SELECT relative_path FROM file_state WHERE page_id = ?1",
            [page_a],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(relative_path, "nested/renamed-a.md");
    let link_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM links WHERE from_page_id = ?1 AND to_page_id = ?2",
            params![page_a, page_b],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(link_count, 1);
}

#[cfg(unix)]
#[test]
fn remap_collection_refuses_phase1_drift_until_new_root_catches_up() {
    let (_db_dir, _db_path, conn) = open_test_db_file();
    let old_root = tempfile::TempDir::new().unwrap();
    let new_root = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(old_root.path().join("notes")).unwrap();
    let collection_id = insert_collection(&conn, "work", old_root.path());
    let original =
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "11111111-1111-7111-8111-111111111111",
        "hello world from note a",
        original,
        "notes/a.md",
    );
    fs::write(old_root.path().join("notes").join("a.md"), original).unwrap();
    fs::create_dir_all(new_root.path().join("notes")).unwrap();
    fs::write(new_root.path().join("notes").join("a.md"), original).unwrap();

    let updated =
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nupdated body before remap";
    fs::write(old_root.path().join("notes").join("a.md"), updated).unwrap();

    let first = remap_collection(&conn, "work", new_root.path(), false).unwrap_err();
    assert!(first.to_string().contains("RemapDriftConflictError"));
    let active_raw_import: Vec<u8> = conn
        .query_row(
            "SELECT raw_bytes FROM raw_imports WHERE page_id = (SELECT id FROM pages WHERE collection_id = ?1 AND slug = 'notes/a') AND is_active = 1",
            [collection_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active_raw_import, updated);

    fs::write(new_root.path().join("notes").join("a.md"), updated).unwrap();
    let second = remap_collection(&conn, "work", new_root.path(), false).unwrap();
    assert_eq!(second.missing_pages, 0);
    assert_eq!(second.mismatched_pages, 0);
    assert_eq!(second.extra_files, 0);
}

#[cfg(not(unix))]
#[test]
fn remap_collection_fails_closed_on_windows_before_mutating_collection_state() {
    let (_db_dir, _db_path, conn) = open_test_db_file();
    let old_root = tempfile::TempDir::new().unwrap();
    let new_root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", old_root.path());
    let original_root = old_root.path().display().to_string();

    let error = remap_collection(&conn, "work", new_root.path(), false).unwrap_err();

    assert!(error
        .to_string()
        .contains("Vault sync commands require Unix"));
    let row: (String, String, i64) = conn
        .query_row(
            "SELECT root_path, state, needs_full_sync FROM collections WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(row.0, original_root);
    assert_eq!(row.1, "active");
    assert_eq!(row.2, 0);
}

#[test]
fn verify_remap_root_uses_unique_hash_fallback_and_ignores_quaidignore_patterns() {
    let conn = open_test_db();
    let old_root = tempfile::TempDir::new().unwrap();
    let new_root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", old_root.path());
    let raw_bytes = b"---\ntitle: Hash Fallback\ntype: concept\n---\nthis body is intentionally long enough to cross the remap hash fallback threshold.\n";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/hash-fallback",
        "11111111-1111-7111-8111-111111111111",
        "this body is intentionally long enough to cross the remap hash fallback threshold.",
        raw_bytes,
        "notes/hash-fallback.md",
    );
    fs::create_dir_all(new_root.path().join("nested")).unwrap();
    fs::write(new_root.path().join("nested").join("moved.md"), raw_bytes).unwrap();
    fs::write(new_root.path().join(".quaidignore"), "ignored/**\n").unwrap();
    fs::create_dir_all(new_root.path().join("ignored")).unwrap();
    fs::write(
        new_root.path().join("ignored").join("secret.md"),
        b"top secret",
    )
    .unwrap();

    let collection = load_collection_by_id(&conn, collection_id).unwrap();
    let summary = verify_remap_root(&conn, &collection, new_root.path()).unwrap();

    assert_eq!(summary.resolved_pages, 1);
    assert_eq!(summary.missing_pages, 0);
    assert_eq!(summary.mismatched_pages, 0);
    assert_eq!(summary.extra_files, 0);
}

#[test]
fn verify_remap_root_ignores_non_markdown_files() {
    let conn = open_test_db();
    let old_root = tempfile::TempDir::new().unwrap();
    let new_root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", old_root.path());
    let raw_bytes =
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "11111111-1111-7111-8111-111111111111",
        "hello world from note a",
        raw_bytes,
        "notes/a.md",
    );
    fs::create_dir_all(new_root.path().join("notes")).unwrap();
    fs::create_dir_all(new_root.path().join("assets")).unwrap();
    fs::write(new_root.path().join("notes").join("a.md"), raw_bytes).unwrap();
    fs::write(
        new_root.path().join("assets").join("logo.png"),
        b"not markdown, not a remap extra",
    )
    .unwrap();

    let collection = load_collection_by_id(&conn, collection_id).unwrap();
    let summary = verify_remap_root(&conn, &collection, new_root.path()).unwrap();

    assert_eq!(summary.resolved_pages, 1);
    assert_eq!(summary.missing_pages, 0);
    assert_eq!(summary.mismatched_pages, 0);
    assert_eq!(summary.extra_files, 0);
}

#[cfg(unix)]
#[test]
fn verify_remap_root_skips_symlinked_entries_in_new_root() {
    use std::os::unix::fs::symlink;

    let conn = open_test_db();
    let old_root = tempfile::TempDir::new().unwrap();
    let new_root = tempfile::TempDir::new().unwrap();
    let linked = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", old_root.path());
    let raw_bytes =
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "11111111-1111-7111-8111-111111111111",
        "hello world from note a",
        raw_bytes,
        "notes/a.md",
    );
    fs::create_dir_all(new_root.path().join("notes")).unwrap();
    fs::write(new_root.path().join("notes").join("a.md"), raw_bytes).unwrap();
    fs::create_dir_all(linked.path().join("shadow")).unwrap();
    fs::write(
        linked.path().join("shadow").join("extra.md"),
        b"reachable only through symlink",
    )
    .unwrap();
    symlink(
        linked.path().join("shadow"),
        new_root.path().join("linked-shadow"),
    )
    .unwrap();

    let collection = load_collection_by_id(&conn, collection_id).unwrap();
    let summary = verify_remap_root(&conn, &collection, new_root.path()).unwrap();

    assert_eq!(summary.resolved_pages, 1);
    assert_eq!(summary.missing_pages, 0);
    assert_eq!(summary.mismatched_pages, 0);
    assert_eq!(summary.extra_files, 0);
}

#[test]
fn verify_remap_root_rejects_invalid_frontmatter_uuid_in_new_root() {
    let conn = open_test_db();
    let old_root = tempfile::TempDir::new().unwrap();
    let new_root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", old_root.path());
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "11111111-1111-7111-8111-111111111111",
        "hello world from note a",
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a",
        "notes/a.md",
    );
    fs::create_dir_all(new_root.path().join("notes")).unwrap();
    fs::write(
        new_root.path().join("notes").join("broken.md"),
        b"---\nmemory_id: definitely-not-a-uuid\n---\nhello world from note a",
    )
    .unwrap();

    let collection = load_collection_by_id(&conn, collection_id).unwrap();
    let error = verify_remap_root(&conn, &collection, new_root.path()).unwrap_err();

    assert!(matches!(error, VaultSyncError::InvariantViolation { .. }));
    assert!(error.to_string().contains("invalid"));
}

#[test]
fn verify_remap_root_does_not_use_hash_fallback_for_short_body() {
    let conn = open_test_db();
    let old_root = tempfile::TempDir::new().unwrap();
    let new_root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", old_root.path());
    let raw_bytes = b"---\ntitle: Short\n---\nshort\n";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/short",
        "11111111-1111-7111-8111-111111111111",
        "short",
        raw_bytes,
        "notes/short.md",
    );
    fs::create_dir_all(new_root.path().join("notes")).unwrap();
    fs::write(new_root.path().join("notes").join("moved.md"), raw_bytes).unwrap();

    let collection = load_collection_by_id(&conn, collection_id).unwrap();
    let error = verify_remap_root(&conn, &collection, new_root.path()).unwrap_err();

    assert!(matches!(
        error,
        VaultSyncError::NewRootVerificationFailed {
            missing: 1,
            mismatched: 0,
            extra: 1,
            ..
        }
    ));
}

#[test]
fn verify_remap_root_rejects_duplicate_hash_candidates_without_uuid_match() {
    let conn = open_test_db();
    let old_root = tempfile::TempDir::new().unwrap();
    let new_root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", old_root.path());
    let raw_bytes = b"---\ntitle: Duplicate Hash\n---\nthis body is intentionally long enough to cross the remap hash fallback threshold.\n";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/hash-duplicate",
        "11111111-1111-7111-8111-111111111111",
        "this body is intentionally long enough to cross the remap hash fallback threshold.",
        raw_bytes,
        "notes/hash-duplicate.md",
    );
    fs::create_dir_all(new_root.path().join("notes")).unwrap();
    fs::write(new_root.path().join("notes").join("one.md"), raw_bytes).unwrap();
    fs::write(new_root.path().join("notes").join("two.md"), raw_bytes).unwrap();

    let collection = load_collection_by_id(&conn, collection_id).unwrap();
    let error = verify_remap_root(&conn, &collection, new_root.path()).unwrap_err();

    assert!(matches!(
        error,
        VaultSyncError::NewRootVerificationFailed {
            missing: 1,
            mismatched: 0,
            extra: 2,
            ..
        }
    ));
}

#[test]
fn verify_remap_root_reports_missing_and_extra_counts() {
    let conn = open_test_db();
    let old_root = tempfile::TempDir::new().unwrap();
    let new_root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", old_root.path());
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "11111111-1111-7111-8111-111111111111",
        "hello world from note a",
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a",
        "notes/a.md",
    );
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/b",
        "22222222-2222-7222-8222-222222222222",
        "hello world from note b",
        b"---\nmemory_id: 22222222-2222-7222-8222-222222222222\n---\nhello world from note b",
        "notes/b.md",
    );
    fs::create_dir_all(new_root.path().join("notes")).unwrap();
    fs::write(
        new_root.path().join("notes").join("a.md"),
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a",
    )
    .unwrap();
    fs::write(
        new_root.path().join("notes").join("extra.md"),
        b"extra file",
    )
    .unwrap();

    let collection = load_collection_by_id(&conn, collection_id).unwrap();
    let error = verify_remap_root(&conn, &collection, new_root.path()).unwrap_err();
    assert!(matches!(
        error,
        VaultSyncError::NewRootVerificationFailed {
            missing: 1,
            mismatched: 0,
            extra: 1,
            ..
        }
    ));
}

#[test]
fn verify_remap_root_error_includes_sampled_diffs() {
    let conn = open_test_db();
    let old_root = tempfile::TempDir::new().unwrap();
    let new_root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", old_root.path());
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "11111111-1111-7111-8111-111111111111",
        "hello world from note a",
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a",
        "notes/a.md",
    );
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/b",
        "22222222-2222-7222-8222-222222222222",
        "hello world from note b",
        b"---\nmemory_id: 22222222-2222-7222-8222-222222222222\n---\nhello world from note b",
        "notes/b.md",
    );
    fs::create_dir_all(new_root.path().join("notes")).unwrap();
    fs::write(
        new_root.path().join("notes").join("a.md"),
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nchanged bytes",
    )
    .unwrap();
    fs::write(
        new_root.path().join("notes").join("extra.md"),
        b"extra file",
    )
    .unwrap();

    let collection = load_collection_by_id(&conn, collection_id).unwrap();
    let error = verify_remap_root(&conn, &collection, new_root.path()).unwrap_err();
    let rendered = error.to_string();

    assert!(rendered.contains("missing_samples=notes/b"));
    assert!(rendered.contains("mismatched_samples=notes/a -> notes/a.md sha256_mismatch"));
    assert!(rendered.contains("extra_samples=notes/extra.md"));
}

#[test]
fn verify_remap_root_reports_mismatched_count_for_duplicate_uuid_candidates() {
    let conn = open_test_db();
    let old_root = tempfile::TempDir::new().unwrap();
    let new_root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", old_root.path());
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "11111111-1111-7111-8111-111111111111",
        "hello world from note a",
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nhello world from note a",
        "notes/a.md",
    );
    fs::create_dir_all(new_root.path().join("notes")).unwrap();
    fs::write(
        new_root.path().join("notes").join("a-one.md"),
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nfirst duplicate",
    )
    .unwrap();
    fs::write(
        new_root.path().join("notes").join("a-two.md"),
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\n---\nsecond duplicate",
    )
    .unwrap();

    let collection = load_collection_by_id(&conn, collection_id).unwrap();
    let error = verify_remap_root(&conn, &collection, new_root.path()).unwrap_err();
    assert!(matches!(
        error,
        VaultSyncError::NewRootVerificationFailed {
            missing: 1,
            mismatched: 1,
            extra: 2,
            ..
        }
    ));
}

#[test]
fn verify_remap_root_source_uses_before_and_after_tree_fences() {
    let source = production_vault_sync_source();
    let start = source.find("pub fn verify_remap_root(").unwrap();
    let end = source[start..]
        .find("pub fn restore_reset(")
        .map(|offset| start + offset)
        .unwrap();
    let snippet = &source[start..end];

    assert!(
        snippet.contains("let before = take_tree_fence(new_root)?;")
            && snippet.contains("let after = take_tree_fence(new_root)?;"),
        "verify_remap_root must fence the tree before and after matching to catch mid-flight drift"
    );
    assert!(
        snippet.contains("return Err(VaultSyncError::NewRootUnstable"),
        "verify_remap_root must fail closed with NewRootUnstableError when the fence changes"
    );
}

#[test]
fn load_new_root_files_source_applies_new_root_ignore_matcher() {
    let source = production_vault_sync_source();
    let start = source.find("fn load_new_root_files(").unwrap();
    let end = source[start..]
        .find("fn resolve_page_matches(")
        .map(|offset| start + offset)
        .unwrap();
    let snippet = &source[start..end];

    assert!(
        snippet.contains("let ignore_globset = build_new_root_ignore_globset(root)?;"),
        "load_new_root_files must build a fresh ignore matcher from the remap root before counting files"
    );
    assert!(
        snippet.contains("if ignore_globset.is_match(relative_path) {"),
        "load_new_root_files must exclude .quaidignore-matched files from remap verification counts"
    );
    assert!(
        snippet.contains("if !is_markdown_file(relative_path) {"),
        "load_new_root_files must reuse the reconciler's Markdown gate so Phase 4 extra counts stay in parity"
    );
}

#[test]
fn resolve_page_matches_source_uses_canonical_resolver_helper() {
    let source = production_vault_sync_source();
    let start = source.find("fn resolve_page_matches(").unwrap();
    let end = source[start..]
        .find("fn take_tree_fence(")
        .map(|offset| start + offset)
        .unwrap();
    let snippet = &source[start..end];

    assert!(
        snippet.contains("resolve_page_identity("),
        "Phase 4 remap verification must invoke the canonical resolve_page_identity helper instead of bespoke matching"
    );
}

#[test]
fn take_tree_fence_source_uses_full_stat_tuple() {
    let source = production_vault_sync_source();
    assert!(
        source.contains("metadata_timestamp_ns(metadata.mtime(), metadata.mtime_nsec())")
            && source.contains("metadata_timestamp_ns(metadata.ctime(), metadata.ctime_nsec())")
            && source.contains("metadata.ino()"),
        "Phase 4 tree fence must capture the full per-file stat tuple so same-size rewrites and atomic replacements cannot slip past remap verification"
    );
}

#[test]
fn walk_tree_source_skips_symlinked_entries() {
    let source = production_vault_sync_source();
    let start = source.find("fn walk_tree(").unwrap();
    let end = source[start..]
        .find("fn path_string(")
        .map(|offset| start + offset)
        .unwrap();
    let snippet = &source[start..end];

    assert!(
        snippet.contains("let file_type = entry.file_type()?;")
            && snippet.contains("if file_type.is_symlink() {"),
        "Phase 4 tree walks must inspect entry file types without following symlinks"
    );
}

#[test]
fn remap_source_runs_safety_pipeline_before_new_root_verification() {
    let source = production_vault_sync_source();
    let remap_start = source.find("pub fn remap_collection(").unwrap();
    let remap_end = source[remap_start..]
        .find("pub fn verify_remap_root(")
        .map(|offset| remap_start + offset)
        .unwrap();
    let remap_source = &source[remap_start..remap_end];

    let safety_idx = remap_source
        .find("run_restore_remap_safety_pipeline_without_mount_check")
        .unwrap();
    let verify_idx = remap_source
        .find("verify_remap_root(conn, &collection, new_root)?")
        .unwrap();
    assert!(
        safety_idx < verify_idx,
        "remap_collection must capture old-root drift before trusting the new root"
    );
}

#[test]
fn remap_online_source_waits_for_exact_ack_before_safety_pipeline() {
    let source = production_vault_sync_source();
    let remap_start = source.find("pub fn remap_collection(").unwrap();
    let remap_end = source[remap_start..]
        .find("pub fn verify_remap_root(")
        .map(|offset| remap_start + offset)
        .unwrap();
    let remap_source = &source[remap_start..remap_end];
    let ack_idx = remap_source
        .find("wait_for_exact_ack(conn, collection.id, &expected_session_id, generation)?")
        .unwrap();
    let safety_idx = remap_source
        .find("run_restore_remap_safety_pipeline_without_mount_check")
        .unwrap();

    assert!(
        ack_idx < safety_idx,
        "online remap must release the live watcher and then capture old-root drift under the acknowledged owner lease"
    );
}

#[test]
fn remap_source_verifies_new_root_before_switching_root_path() {
    let source = production_vault_sync_source();
    let remap_start = source.find("pub fn remap_collection(").unwrap();
    let remap_end = source[remap_start..]
        .find("pub fn verify_remap_root(")
        .map(|offset| remap_start + offset)
        .unwrap();
    let remap_source = &source[remap_start..remap_end];
    let verify_idx = remap_source
        .find("verify_remap_root(conn, &collection, new_root)?")
        .unwrap();
    let update_idx = remap_source.find("SET root_path = ?2,").unwrap();

    assert!(
        verify_idx < update_idx,
        "remap_collection must prove the target tree before rewriting the collection root"
    );
}

#[test]
fn remap_offline_source_uses_short_lived_lease_and_inline_attach() {
    let source = production_vault_sync_source();
    let remap_start = source.find("pub fn remap_collection(").unwrap();
    let remap_end = source[remap_start..]
        .find("pub fn verify_remap_root(")
        .map(|offset| remap_start + offset)
        .unwrap();
    let remap_source = &source[remap_start..remap_end];
    let offline_start = remap_source
        .find("let lease = start_short_lived_owner_lease")
        .unwrap();
    let offline_end = remap_source[offline_start..]
        .find("Ok(summary)")
        .map(|offset| offline_start + offset)
        .unwrap();
    let offline_source = &remap_source[offline_start..offline_end];

    assert!(
        offline_source.contains("complete_attach(")
            && offline_source.contains("AttachReason::RemapPostReconcile"),
        "offline remap must run the attach/full-hash path inline while the CLI lease is live"
    );
    assert!(
        !offline_source.contains("unregister_session("),
        "offline remap must not drop its lease before the inline attach finishes"
    );
}
