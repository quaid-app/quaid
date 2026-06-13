#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! SIGKILL-mid-write recovery harness.
//!
//! A `quaid put` writes its target file under a `*.needs_full_sync` recovery
//! sentinel and only removes the sentinel after the SQLite commit succeeds (see
//! `src/commands/put.rs` and `src/core/vault_sync/recovery.rs`). If the process
//! dies between the rename and the commit, the sentinel survives, and the next
//! daemon startup (`start_serve_runtime` → `recover_owned_collection_sentinels`)
//! must flag the collection `needs_full_sync`, run a full-hash recovery that
//! reconciles the DB to whatever bytes are on disk, then delete the sentinel.
//!
//! This file pins that recovery two ways:
//!
//! 1. `sigkill_mid_put_leaves_db_and_file_coherent_after_recovery` does a *real*
//!    `kill -9` of a `quaid put` subprocess and asserts that after a recovery
//!    pass the DB row and the on-disk file agree, whatever phase the kill hit.
//! 2. `startup_recovery_reconciles_db_to_disk_and_clears_sentinel` reconstructs
//!    the exact on-disk state a SIGKILL leaves between rename and commit (a
//!    leftover sentinel plus a file newer than the DB row) and drives the
//!    production `start_serve_runtime` recovery deterministically.
//!
//! NB: the prompt's `PutTestHooks` blocking hooks are `#[cfg(all(test, unix))]`
//! and live in an in-process static map, so they are NOT compiled into the
//! `quaid` binary and cannot park a *subprocess* put at the staged point.
//! Test (2) therefore reconstructs the staged-crash artifacts directly, which
//! exercises the identical production recovery code (`start_serve_runtime`)
//! that an in-crate hook-driven crash would.

#[path = "common/mod.rs"]
mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;
#[path = "common/vault_sync_fixtures.rs"]
mod fixtures;

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rusqlite::Connection;

#[cfg(target_os = "linux")]
use fixtures::EnvVarGuard;
use fixtures::{
    create_startup_recovery_sentinel, env_mutation_lock, insert_collection,
    insert_page_with_raw_import, open_test_db_file, write_restore_file,
};
use quaid::core::vault_sync::{recovery_root_for_db_path, start_serve_runtime};

fn poll<T>(timeout: Duration, mut probe: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = probe() {
            return Some(value);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn page_row(db_path: &str, slug: &str) -> Option<(String, i64)> {
    let conn = Connection::open(db_path).ok()?;
    conn.query_row(
        "SELECT compiled_truth, version FROM pages WHERE slug = ?1",
        [slug],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .ok()
}

#[cfg(unix)]
#[test]
fn sigkill_mid_put_leaves_db_and_file_coherent_after_recovery() {
    let _lock = env_mutation_lock().lock().unwrap();
    #[cfg(target_os = "linux")]
    let _runtime_root = fixtures::secure_runtime_root();
    #[cfg(target_os = "linux")]
    let _xdg = EnvVarGuard::set("XDG_RUNTIME_DIR", _runtime_root.path().to_str().unwrap());

    let (dir, db_path, conn) = open_test_db_file();
    let root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", root.path());
    let canonical_root = std::fs::canonicalize(root.path()).unwrap();
    // Make the default write-target point elsewhere so the bare-slug create
    // routes to our "work" collection unambiguously by using an explicit slug.
    let uuid = "01969f11-9448-7d79-8d3f-c68f54760001";
    let seed = format!(
        "---\nmemory_id: {uuid}\nslug: notes/kill\ntitle: Kill\ntype: concept\n---\nseed body before the crash.\n"
    );
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/kill",
        uuid,
        "seed body before the crash.",
        seed.as_bytes(),
        "notes/kill.md",
    );
    write_restore_file(&canonical_root, "notes/kill.md", seed.as_bytes());
    drop(conn);

    // Spawn a real put and SIGKILL it. Whatever phase the kill catches, the
    // DB+file must end coherent after a recovery pass — never a torn row.
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    command
        .arg("--db")
        .arg(&db_path)
        .args(["put", "work::notes/kill", "--expected-version", "1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().expect("spawn quaid put");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(
            format!(
                "---\nmemory_id: {uuid}\nslug: notes/kill\ntitle: Kill\ntype: concept\n---\nbody written just before SIGKILL.\n"
            )
            .as_bytes(),
        )
        .expect("write child stdin");
    // kill() is SIGKILL on Unix — the writer cannot run any cleanup.
    child.kill().expect("kill -9 the put");
    let _ = child.wait();

    // Drive production recovery: a fresh serve runtime reconciles the DB to the
    // on-disk file and consumes any leftover sentinel.
    let runtime = start_serve_runtime(db_path.clone()).expect("start serve runtime");

    let on_disk = std::fs::read_to_string(canonical_root.join("notes").join("kill.md"))
        .expect("file on disk");
    let coherent = poll(Duration::from_secs(8), || {
        let (truth, _version) = page_row(&db_path, "notes/kill")?;
        // Coherence: the compiled_truth the DB holds must match the body bytes
        // currently on disk (recovery reconciles DB → disk).
        on_disk.contains(truth.trim()).then_some(())
    });
    drop(runtime);

    assert!(
        coherent.is_some(),
        "after SIGKILL + recovery, the DB row and the on-disk file must agree.\non disk:\n{on_disk}"
    );
    // The recovery sentinel directory must not retain a dangling sentinel.
    assert_eq!(
        leftover_sentinels(&db_path, collection_id),
        0,
        "recovery must consume any leftover put sentinel"
    );

    drop(dir);
}

#[cfg(unix)]
#[test]
fn startup_recovery_reconciles_db_to_disk_and_clears_sentinel() {
    let _lock = env_mutation_lock().lock().unwrap();
    #[cfg(target_os = "linux")]
    let _runtime_root = fixtures::secure_runtime_root();
    #[cfg(target_os = "linux")]
    let _xdg = EnvVarGuard::set("XDG_RUNTIME_DIR", _runtime_root.path().to_str().unwrap());

    let (dir, db_path, conn) = open_test_db_file();
    let root = tempfile::TempDir::new().unwrap();
    let collection_id = insert_collection(&conn, "work", root.path());
    let canonical_root = std::fs::canonicalize(root.path()).unwrap();
    let uuid = "01969f11-9448-7d79-8d3f-c68f54760002";

    // DB row + raw_import hold the OLD body (the commit never landed).
    let old = format!(
        "---\nmemory_id: {uuid}\nslug: notes/staged\ntitle: Staged\ntype: concept\n---\nold body still in the database.\n"
    );
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/staged",
        uuid,
        "old body still in the database.",
        old.as_bytes(),
        "notes/staged.md",
    );

    // On disk: the NEW body the renamed-but-uncommitted put left behind.
    let new = format!(
        "---\nmemory_id: {uuid}\nslug: notes/staged\ntitle: Staged\ntype: concept\n---\nnew body the crashed put renamed into place.\n"
    );
    write_restore_file(&canonical_root, "notes/staged.md", new.as_bytes());

    // The leftover recovery sentinel a SIGKILL between rename and commit leaves.
    let recovery_root = recovery_root_for_db_path(Path::new(&db_path));
    create_startup_recovery_sentinel(
        &recovery_root,
        collection_id,
        "crashed-write.needs_full_sync",
    );
    drop(conn);

    let runtime = start_serve_runtime(db_path.clone()).expect("start serve runtime");

    let recovered = poll(Duration::from_secs(8), || {
        let conn = Connection::open(&db_path).ok()?;
        let (state, needs_full_sync, truth): (String, i64, String) = conn
            .query_row(
                "SELECT c.state, c.needs_full_sync, p.compiled_truth
                 FROM collections c
                 JOIN pages p ON p.collection_id = c.id AND p.slug = 'notes/staged'
                 WHERE c.id = ?1",
                [collection_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok()?;
        (state == "active"
            && needs_full_sync == 0
            && truth == "new body the crashed put renamed into place."
            && leftover_sentinels(&db_path, collection_id) == 0)
            .then_some(truth)
    });
    drop(runtime);

    assert!(
        recovered.is_some(),
        "startup recovery must reconcile the DB to the on-disk file, clear needs_full_sync, \
         and delete the sentinel"
    );

    drop(dir);
}

#[cfg(unix)]
fn leftover_sentinels(db_path: &str, collection_id: i64) -> usize {
    let recovery_root = recovery_root_for_db_path(Path::new(db_path));
    let dir = quaid::core::vault_sync::collection_recovery_dir(&recovery_root, collection_id);
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(".needs_full_sync")
                })
                .count()
        })
        .unwrap_or(0)
}
