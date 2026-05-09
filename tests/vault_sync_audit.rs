#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::too_many_lines,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Full-hash audit batching and ttl tests.
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

#[cfg(unix)]
#[test]
fn full_hash_audit_pass_rehashes_due_active_lease_collections_only() {
    let conn = open_test_db();
    let due_root = tempfile::TempDir::new().unwrap();
    let skipped_root = tempfile::TempDir::new().unwrap();
    let due_id = insert_collection(&conn, "due", due_root.path());
    let skipped_id = insert_collection(&conn, "skipped", skipped_root.path());

    let due_uuid = Uuid::now_v7().to_string();
    let due_bytes = format!(
        "---\nmemory_id: {due_uuid}\nslug: notes/due\ntitle: Due\ntype: concept\n---\nBody.\n"
    );
    fs::create_dir_all(due_root.path().join("notes")).unwrap();
    fs::write(due_root.path().join("notes/due.md"), due_bytes.as_bytes()).unwrap();
    insert_page_with_raw_import(
        &conn,
        due_id,
        "notes/due",
        &due_uuid,
        "Body.",
        due_bytes.as_bytes(),
        "notes/due.md",
    );
    conn.execute(
        "UPDATE file_state
         SET last_full_hash_at = datetime('now', '-8 days')
         WHERE collection_id = ?1",
        [due_id],
    )
    .unwrap();

    let skipped_uuid = Uuid::now_v7().to_string();
    let skipped_bytes = format!(
        "---\nmemory_id: {skipped_uuid}\nslug: notes/skipped\ntitle: Skipped\ntype: concept\n---\nBody.\n"
    );
    fs::create_dir_all(skipped_root.path().join("notes")).unwrap();
    fs::write(
        skipped_root.path().join("notes/skipped.md"),
        skipped_bytes.as_bytes(),
    )
    .unwrap();
    insert_page_with_raw_import(
        &conn,
        skipped_id,
        "notes/skipped",
        &skipped_uuid,
        "Body.",
        skipped_bytes.as_bytes(),
        "notes/skipped.md",
    );
    conn.execute(
        "UPDATE file_state
         SET last_full_hash_at = datetime('now', '-1 days')
         WHERE collection_id = ?1",
        [skipped_id],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
         VALUES ('serve-audit', 1, 'host', datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-audit')",
        [due_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-audit')",
        [skipped_id],
    )
    .unwrap();
    conn.execute(
        "UPDATE collections SET active_lease_session_id = 'serve-audit' WHERE id IN (?1, ?2)",
        rusqlite::params![due_id, skipped_id],
    )
    .unwrap();

    let audited = run_full_hash_audit_pass(&conn, "serve-audit").unwrap();

    assert_eq!(audited.len(), 1);
    assert_eq!(audited[0].0, due_id);
    assert_eq!(audited[0].1, "due");
}

#[cfg(unix)]
#[test]
fn full_hash_audit_pass_limits_each_cycle_to_a_daily_subset() {
    let _guard = env_mutation_lock().lock().unwrap();
    let _env = EnvVarGuard::set("QUAID_FULL_HASH_AUDIT_DAYS", "3");
    let conn = open_test_db();
    let root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", root.path());

    fs::create_dir_all(root.path().join("notes")).unwrap();
    for index in 0..7 {
        let relative_path = format!("notes/{index:02}.md");
        let slug = format!("notes/{index:02}");
        let uuid = Uuid::now_v7().to_string();
        let raw = format!(
            "---\nmemory_id: {uuid}\nslug: {slug}\ntitle: Note {index}\ntype: concept\n---\nBody {index}.\n"
        );
        fs::write(root.path().join(&relative_path), raw.as_bytes()).unwrap();
        insert_page_with_raw_import(
            &conn,
            collection_id,
            &slug,
            &uuid,
            &format!("Body {index}."),
            raw.as_bytes(),
            &relative_path,
        );
    }
    conn.execute(
        "UPDATE file_state
         SET last_full_hash_at = datetime('now', '-8 days')
         WHERE collection_id = ?1",
        [collection_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO serve_sessions (session_id, pid, host, heartbeat_at)
         VALUES ('serve-audit', 1, 'host', datetime('now'))",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO collection_owners (collection_id, session_id) VALUES (?1, 'serve-audit')",
        [collection_id],
    )
    .unwrap();
    conn.execute(
        "UPDATE collections SET active_lease_session_id = 'serve-audit' WHERE id = ?1",
        [collection_id],
    )
    .unwrap();

    let audited = run_full_hash_audit_pass(&conn, "serve-audit").unwrap();

    assert_eq!(audited.len(), 1);
    assert_eq!(
        audited[0].2.walked, 3,
        "7 files over 3 days should hash only ceil(7/3) files per cycle"
    );
    assert_eq!(audited[0].2.unchanged, 3);
    let fresh_paths = conn
        .prepare(
            "SELECT relative_path
             FROM file_state
             WHERE collection_id = ?1
               AND last_full_hash_at > datetime('now', '-1 minute')
             ORDER BY relative_path ASC",
        )
        .unwrap()
        .query_map([collection_id], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let stale_paths = conn
        .prepare(
            "SELECT relative_path
             FROM file_state
             WHERE collection_id = ?1
               AND last_full_hash_at < datetime('now', '-7 days')
             ORDER BY relative_path ASC",
        )
        .unwrap()
        .query_map([collection_id], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        fresh_paths,
        vec![
            "notes/00.md".to_owned(),
            "notes/01.md".to_owned(),
            "notes/02.md".to_owned()
        ],
        "scheduled audit must update only the first budgeted subset this cycle"
    );
    assert_eq!(stale_paths.len(), 4);

    let audited_again = run_full_hash_audit_pass(&conn, "serve-audit").unwrap();

    assert_eq!(audited_again.len(), 1);
    assert_eq!(
        audited_again[0].2.walked, 3,
        "the next serve-loop cycle must stay bounded to the same daily subset size"
    );
    let remaining_stale = conn
        .prepare(
            "SELECT relative_path
             FROM file_state
             WHERE collection_id = ?1
               AND last_full_hash_at < datetime('now', '-7 days')
             ORDER BY relative_path ASC",
        )
        .unwrap()
        .query_map([collection_id], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        remaining_stale,
        vec!["notes/06.md".to_owned()],
        "scheduled audit must advance to the next oldest subset instead of re-running a whole-vault pass"
    );
}

#[test]
fn run_full_hash_audit_pass_batches_due_rows_instead_of_inline_full_vault_reconcile() {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("core")
            .join("vault_sync.rs"),
    )
    .unwrap();
    let fn_start = source
        .find("pub fn run_full_hash_audit_pass(")
        .expect("run_full_hash_audit_pass fn present");
    let fn_end = source[fn_start..]
        .find("pub fn audit_collection(")
        .map(|offset| fn_start + offset)
        .expect("audit_collection fn follows run_full_hash_audit_pass");
    let fn_body = &source[fn_start..fn_end];

    assert!(
        fn_body.contains("scheduled_full_hash_audit_budget(total_files.max(0) as usize)"),
        "scheduled audit must compute a bounded per-cycle budget before hashing overdue rows"
    );
    assert!(
        fn_body.contains("LIMIT ?3"),
        "scheduled audit must select only the budgeted overdue rows per serve-loop cycle"
    );
    assert!(
        fn_body.contains("scheduled_full_hash_audit_authorized("),
        "scheduled audit must hash only the selected overdue subset"
    );
    assert!(
        !fn_body.contains("full_hash_reconcile_authorized("),
        "run_full_hash_audit_pass must not collapse back into a whole-vault inline reconcile"
    );
}
