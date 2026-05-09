#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::too_many_lines,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Collection watcher, supervisor, and `plain_sync` reconcile tests.
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

#[test]
fn sync_collection_watchers_production_logic_stays_active_only_and_generation_aware() {
    let source = production_vault_sync_source();
    let start = source.find("fn sync_collection_watchers(").unwrap();
    let end = source[start..]
        .find("fn run_watcher_reconcile(")
        .map(|offset| start + offset)
        .unwrap();
    let snippet = &source[start..end];

    assert!(
        snippet.contains("detach_active_collections_with_empty_root_path(conn)?;"),
        "watcher sync must normalize empty root paths before watching: {snippet}"
    );
    assert!(
        snippet.contains("WHERE state = 'active'"),
        "watcher sync must only enumerate active collections: {snippet}"
    );
    assert!(
        snippet.contains("watchers.retain(|collection_id, _| active.contains_key(collection_id))"),
        "watcher sync must drop watchers for non-active collections: {snippet}"
    );
    assert!(
        snippet.contains("state.root_path != root_path")
            && snippet.contains("state.generation != generation"),
        "watcher sync must replace watchers when root or reload_generation changes: {snippet}"
    );
    assert!(
        snippet.contains("state.generation = generation;"),
        "replacement watchers must inherit the new reload_generation: {snippet}"
    );
}

#[test]
fn run_overflow_recovery_pass_production_logic_uses_active_lease_and_active_gate() {
    let source = production_vault_sync_source();
    let start = source.find("fn run_overflow_recovery_pass(").unwrap();
    let end = source[start..]
        .find("pub fn start_serve_runtime(")
        .map(|offset| start + offset)
        .unwrap();
    let snippet = &source[start..end];

    assert!(
        snippet.contains("WHERE state = 'active' AND needs_full_sync = 1"),
        "overflow recovery must gate itself to active collections: {snippet}"
    );
    assert!(
        snippet.contains("FullHashReconcileMode::OverflowRecovery"),
        "overflow recovery must use the repaired mode label: {snippet}"
    );
    assert!(
        snippet.contains("FullHashReconcileAuthorization::ActiveLease"),
        "overflow recovery must reuse the active lease authorization: {snippet}"
    );
    assert!(
        snippet.contains("overflow_recovery_skipped_lease_mismatch"),
        "overflow recovery must warn on lease mismatch rather than bypassing ownership: {snippet}"
    );
}

#[test]
fn start_collection_watcher_production_logic_keeps_native_first_poll_fallback() {
    let source = production_vault_sync_source();
    let start = source.find("fn start_collection_watcher(").unwrap();
    let end = source[start..]
        .find("fn sync_collection_watchers(")
        .map(|offset| start + offset)
        .unwrap();
    let snippet = &source[start..end];

    assert!(
        snippet.contains("watcher_native_init_failed"),
        "native watcher failures must warn before fallback: {snippet}"
    );
    assert!(
        snippet.contains("PollWatcher::new"),
        "native watcher failures must fall back to poll mode: {snippet}"
    );
    assert!(
        snippet.contains("WatcherMode::Poll"),
        "fallback path must record poll mode explicitly: {snippet}"
    );
}

#[test]
fn watcher_supervisor_production_logic_tracks_crash_state_and_backoff() {
    let source = production_vault_sync_source();
    let start = source.find("fn mark_watcher_crashed(").unwrap();
    let end = source[start..]
        .find("fn publish_watcher_health(")
        .map(|offset| start + offset)
        .unwrap();
    let snippet = &source[start..end];

    assert!(
        snippet.contains("state.mode = WatcherMode::Crashed;"),
        "watcher crashes must be surfaced as an explicit crashed mode: {snippet}"
    );
    assert!(
        snippet.contains("state.backoff_until = Some(now + backoff);"),
        "watcher crashes must record restart backoff: {snippet}"
    );
    assert!(
        snippet.contains("watcher_backoff_duration"),
        "watcher crashes must use exponential backoff helper: {snippet}"
    );
}

#[cfg(unix)]
#[test]
fn plain_sync_reconciles_active_root_and_clears_needs_full_sync() {
    let (_dir, _db_path, conn) = open_test_db_file();
    let root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", root.path());
    conn.execute(
        "UPDATE collections SET needs_full_sync = 1 WHERE id = ?1",
        [collection_id],
    )
    .unwrap();
    fs::write(
        root.path().join("note.md"),
        "---\nslug: notes/note\ntitle: Note\ntype: concept\n---\nA body long enough to reconcile through the active-root path.\n",
    )
    .unwrap();

    let stats = sync_collection(&conn, "work").unwrap();
    let row: (String, i64, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT state, needs_full_sync, last_sync_at, active_lease_session_id
             FROM collections WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    let page_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE collection_id = ?1",
            [collection_id],
            |row| row.get(0),
        )
        .unwrap();

    assert_eq!(stats.walked, 1);
    assert_eq!(row.0, "active");
    assert_eq!(row.1, 0);
    assert!(row.2.is_some());
    assert!(row.3.is_none());
    assert_eq!(page_count, 1);
}

#[cfg(unix)]
#[test]
fn plain_sync_turns_duplicate_uuid_into_terminal_reconcile_halt() {
    let (_dir, _db_path, conn) = open_test_db_file();
    let root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", root.path());
    let uuid = "01969f11-9448-7d79-8d3f-c68f54768888";
    let note_a = format!(
        "---\nmemory_id: {uuid}\nslug: notes/a\ntitle: A\ntype: concept\n---\nThis body is comfortably above the minimum size for rename inference.\n"
    );
    let note_b = format!(
        "---\nmemory_id: {uuid}\nslug: notes/b\ntitle: B\ntype: concept\n---\nThis second body is also comfortably above the minimum size for rename inference.\n"
    );
    fs::write(root.path().join("a.md"), note_a).unwrap();
    fs::write(root.path().join("b.md"), note_b).unwrap();

    let error = sync_collection(&conn, "work").unwrap_err().to_string();
    let row: (Option<String>, Option<String>, i64) = conn
        .query_row(
            "SELECT reconcile_halted_at, reconcile_halt_reason,
                    (SELECT COUNT(*) FROM pages WHERE collection_id = ?1)
             FROM collections WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();

    assert!(error.contains("ReconcileHaltedError"));
    assert!(error.contains("DuplicateUuidError"));
    assert!(row.0.is_some());
    assert_eq!(row.1.as_deref(), Some("duplicate_uuid"));
    assert_eq!(row.2, 0);
}

#[cfg(unix)]
#[test]
fn plain_sync_turns_trivial_hash_ambiguity_into_terminal_reconcile_halt() {
    let (_dir, _db_path, conn) = open_test_db_file();
    let root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", root.path());
    let content = concat!(
        "---\n",
        "slug: notes/template\n",
        "title: Template Note\n",
        "type: concept\n",
        "meta: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
        "---\n",
        "Hi\n",
    );
    fs::write(root.path().join("template.md"), content).unwrap();
    let stat = quaid::core::file_state::stat_file(&root.path().join("template.md")).unwrap();
    let sha256 = quaid::core::file_state::hash_file(&root.path().join("template.md")).unwrap();
    conn.execute(
        "INSERT INTO pages (collection_id, slug, uuid, type, title, compiled_truth, timeline)
         VALUES (?1, 'notes/template', ?2, 'concept', 'Template', 'Hi', '')",
        params![collection_id, "01969f11-9448-7d79-8d3f-c68f54767777"],
    )
    .unwrap();
    let page_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO file_state
             (collection_id, relative_path, page_id, mtime_ns, ctime_ns, size_bytes, inode, sha256)
         VALUES (?1, 'old-template.md', ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            collection_id,
            page_id,
            stat.mtime_ns,
            stat.ctime_ns,
            stat.size_bytes,
            stat.inode,
            sha256
        ],
    )
    .unwrap();
    quaid::core::raw_imports::rotate_active_raw_import(
        &conn,
        page_id,
        "old-template.md",
        content.as_bytes(),
    )
    .unwrap();

    let error = sync_collection(&conn, "work").unwrap_err().to_string();
    let row: (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT reconcile_halted_at, reconcile_halt_reason
             FROM collections WHERE id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    assert!(error.contains("ReconcileHaltedError"));
    assert!(error.contains("UnresolvableTrivialContentError"));
    assert!(row.0.is_some());
    assert_eq!(row.1.as_deref(), Some("unresolvable_trivial_content"));
}
