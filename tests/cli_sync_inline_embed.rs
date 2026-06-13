#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "integration tests panic on setup failure and print diagnostics"
)]

//! CLI write-path inline embedding drain (review item #10, step 3 — the DAB
//! #220 "footgun"): a CLI-only `collection sync` (no running serve daemon) must
//! leave the just-reconciled pages semantically searchable, and `--no-embed`
//! must opt out and leave the queue pending.

#![cfg(unix)]

use std::fs;

use quaid::commands::collection::{run, CollectionAction, CollectionAddArgs, CollectionSyncArgs};
use quaid::core::db;
use quaid::core::inference::search_vec;
use rusqlite::Connection;

fn open_test_db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    (dir, conn)
}

fn pending_jobs(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM embedding_jobs WHERE job_state IN ('pending', 'failed', 'running')",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

fn attach_with_page(conn: &Connection, vault: &std::path::Path, body: &str) {
    fs::create_dir_all(vault.join("notes")).unwrap();
    fs::write(
        vault.join("notes").join("topic.md"),
        format!("---\ntitle: Topic\ntype: concept\n---\n## State\n{body}\n"),
    )
    .unwrap();
    run(
        conn,
        CollectionAction::Add(CollectionAddArgs {
            name: "work".to_owned(),
            path: vault.to_path_buf(),
            read_only: false,
            writable: true,
            write_quaid_id: false,
            namespace: None,
        }),
        true,
    )
    .unwrap();
}

#[test]
fn cli_sync_without_daemon_embeds_new_pages() {
    let (_db_dir, conn) = open_test_db();
    let vault = tempfile::TempDir::new().unwrap();
    attach_with_page(
        &conn,
        vault.path(),
        "distributed consensus and quorum replication",
    );

    // Attach reconciled the file and enqueued an embedding job, but nothing has
    // drained it yet (no daemon).
    assert!(
        pending_jobs(&conn) >= 1,
        "attach must enqueue an embedding job that is not yet drained"
    );
    assert!(
        search_vec("consensus replication", 5, None, None, &conn)
            .unwrap()
            .is_empty(),
        "semantic search must be empty before any drain (the footgun)"
    );

    // A plain CLI sync (no daemon) drains the queue inline.
    run(
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
    .unwrap();

    assert_eq!(
        pending_jobs(&conn),
        0,
        "sync must drain the embedding queue inline"
    );
    let results = search_vec("consensus replication", 5, None, None, &conn).unwrap();
    assert!(
        results.iter().any(|r| r.slug == "notes/topic"),
        "sync without a daemon must make the new page semantically searchable: {results:?}"
    );
}

#[test]
fn cli_sync_no_embed_leaves_queue_pending() {
    let (_db_dir, conn) = open_test_db();
    let vault = tempfile::TempDir::new().unwrap();
    attach_with_page(&conn, vault.path(), "supply chain logistics optimization");

    let before = pending_jobs(&conn);
    assert!(before >= 1);

    run(
        &conn,
        CollectionAction::Sync(CollectionSyncArgs {
            name: "work".to_owned(),
            remap_root: None,
            finalize_pending: false,
            online: false,
            no_embed: true,
        }),
        true,
    )
    .unwrap();

    assert_eq!(
        pending_jobs(&conn),
        before,
        "--no-embed must leave the embedding queue pending"
    );
    assert!(
        search_vec("logistics optimization", 5, None, None, &conn)
            .unwrap()
            .is_empty(),
        "--no-embed must not produce semantic results"
    );
}
