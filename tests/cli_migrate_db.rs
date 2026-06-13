#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Subprocess tests for `quaid migrate`: the explicit schema-migration
//! ladder. Fixtures are real v9 databases built from the v0.21.0 DDL
//! (`tests/fixtures/schema_v9.sql`, captured via
//! `git show v0.21.0:src/schema.sql`).

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rusqlite::Connection;
use serde_json::Value;

const SCHEMA_V9: &str = include_str!("fixtures/schema_v9.sql");

fn run_quaid(home: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .args(args)
        .output()
        .expect("run quaid")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Builds a populated schema-v9 database: three pages, a duplicated
/// `programmatic` link (must survive), a duplicated `wiki_link` derived edge
/// (must collapse to one row), and one distinct `wiki_link` edge.
fn build_v9_fixture(db_path: &Path, vault_root: &Path) {
    fs::create_dir_all(vault_root).unwrap();
    let conn = Connection::open(db_path).unwrap();
    conn.execute_batch(SCHEMA_V9).unwrap();
    conn.execute_batch("PRAGMA user_version = 9;").unwrap();

    conn.execute_batch(
        "INSERT INTO quaid_config (key, value) VALUES
             ('model_id',       'BAAI/bge-small-en-v1.5'),
             ('model_alias',    'small'),
             ('embedding_dim',  '384'),
             ('schema_version', '9');",
    )
    .unwrap();

    conn.execute(
        "INSERT INTO collections (id, name, root_path, state, writable, is_write_target)
         VALUES (1, 'default', ?1, 'active', 1, 1)",
        [vault_root.to_str().unwrap()],
    )
    .unwrap();

    conn.execute_batch(
        "INSERT INTO pages (id, collection_id, slug, uuid, type, title, compiled_truth, wing) VALUES
             (1, 1, 'people/alice',   '018f0000-0000-7000-8000-000000000001', 'person',  'Alice',   'Alice runs ops.', 'people'),
             (2, 1, 'people/bob',     '018f0000-0000-7000-8000-000000000002', 'person',  'Bob',     'Bob writes Rust.', 'people'),
             (3, 1, 'projects/quaid', '018f0000-0000-7000-8000-000000000003', 'project', 'Quaid',   'Personal memory.', 'projects');

         INSERT INTO links (id, from_page_id, to_page_id, relationship, source_kind) VALUES
             (1, 1, 2, 'related',  'programmatic'),
             (2, 1, 2, 'related',  'programmatic'),
             (3, 1, 3, 'related',  'wiki_link'),
             (4, 1, 3, 'related',  'wiki_link'),
             (5, 2, 3, 'mentions', 'wiki_link');",
    )
    .unwrap();
}

fn stored_versions(db_path: &Path) -> (String, String) {
    let conn = Connection::open(db_path).unwrap();
    let quaid_config: String = conn
        .query_row(
            "SELECT value FROM quaid_config WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let legacy: String = conn
        .query_row(
            "SELECT value FROM config WHERE key = 'version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    (quaid_config, legacy)
}

fn temp_layout(dir: &tempfile::TempDir) -> (PathBuf, PathBuf, PathBuf) {
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let db_path = dir.path().join("memory.db");
    let vault_root = dir.path().join("vault");
    (home, db_path, vault_root)
}

#[test]
fn plain_open_fail_closes_on_v9_database() {
    let dir = tempfile::TempDir::new().unwrap();
    let (home, db_path, vault_root) = temp_layout(&dir);
    build_v9_fixture(&db_path, &vault_root);

    let output = run_quaid(&home, &["--db", db_path.to_str().unwrap(), "list"]);

    assert!(
        !output.status.success(),
        "plain open of a v9 database must fail closed\nstdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Found version 9, expected 10"),
        "stderr should name the version mismatch, got:\n{stderr}"
    );

    // Fail-closed means untouched: still v9, no backup written.
    let (quaid_config, legacy) = stored_versions(&db_path);
    assert_eq!(quaid_config, "9");
    assert_eq!(legacy, "9");
    assert!(!PathBuf::from(format!("{}.bak", db_path.display())).exists());
}

#[test]
fn migrate_upgrades_v9_database_to_current_schema_with_data_intact() {
    let dir = tempfile::TempDir::new().unwrap();
    let (home, db_path, vault_root) = temp_layout(&dir);
    build_v9_fixture(&db_path, &vault_root);
    let db = db_path.to_str().unwrap();

    let output = run_quaid(&home, &["--db", db, "migrate"]);
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("from schema version 9 to 10"),
        "report should describe the ladder run, got:\n{stdout}"
    );
    assert!(stdout.contains("Integrity check: ok"));

    // Backup written before the first step.
    let backup = PathBuf::from(format!("{db}.bak"));
    assert!(backup.exists(), "pre-migration backup must exist");
    let backup_conn = Connection::open(&backup).unwrap();
    let backup_version: String = backup_conn
        .query_row(
            "SELECT value FROM quaid_config WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(backup_version, "9", "backup must remain a v9 snapshot");

    let (quaid_config, legacy) = stored_versions(&db_path);
    assert_eq!(quaid_config, "10");
    assert_eq!(legacy, "10");

    let conn = Connection::open(&db_path).unwrap();
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");

    // links gained the edge_weight column and the partial unique index.
    let has_edge_weight: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('links') WHERE name = 'edge_weight'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(has_edge_weight, 1);
    let has_unique_index: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'index' AND name = 'idx_links_unique_derived_edge'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(has_unique_index, 1);

    // The extended CHECK accepts the new derived source kinds.
    conn.execute(
        "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind, edge_weight)
         VALUES (3, 1, 'related', 'frontmatter', 1.0)",
        [],
    )
    .unwrap();
    conn.execute("DELETE FROM links WHERE source_kind = 'frontmatter'", [])
        .unwrap();

    // Data intact: pages untouched, programmatic duplicates preserved,
    // wiki_link duplicates collapsed to the oldest row.
    let pages: i64 = conn
        .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(pages, 3);
    let programmatic: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM links WHERE source_kind = 'programmatic'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(programmatic, 2);
    let wiki_rows: Vec<i64> = conn
        .prepare("SELECT id FROM links WHERE source_kind = 'wiki_link' ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(wiki_rows, vec![3, 5]);

    // v10 graph config defaults are seeded.
    let graph_depth: String = conn
        .query_row(
            "SELECT value FROM config WHERE key = 'graph_depth'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(graph_depth, "0");
    drop(conn);

    // The migrated database opens and works with the current binary.
    let list = run_quaid(&home, &["--db", db, "list"]);
    assert_success(&list);
    let listed = String::from_utf8_lossy(&list.stdout);
    assert!(
        listed.contains("people/alice"),
        "migrated data should be listable, got:\n{listed}"
    );
}

#[test]
fn migrate_reports_json_when_requested() {
    let dir = tempfile::TempDir::new().unwrap();
    let (home, db_path, vault_root) = temp_layout(&dir);
    build_v9_fixture(&db_path, &vault_root);
    let db = db_path.to_str().unwrap();

    let output = run_quaid(&home, &["--json", "--db", db, "migrate"]);
    assert_success(&output);

    let report: Value = serde_json::from_slice(&output.stdout).expect("valid JSON report");
    assert_eq!(report["from_version"], 9);
    assert_eq!(report["to_version"], 10);
    assert_eq!(report["steps_applied"], serde_json::json!([10]));
    assert_eq!(report["integrity_check"], "ok");
    assert_eq!(report["backup_path"], format!("{db}.bak"));
    assert_eq!(report["pages"]["before"], 3);
    assert_eq!(report["pages"]["after"], 3);
    assert_eq!(report["links"]["before"], 5);
    assert_eq!(report["links"]["after"], 4);
}

#[test]
fn migrate_is_noop_on_already_current_database() {
    let dir = tempfile::TempDir::new().unwrap();
    let (home, db_path, _vault_root) = temp_layout(&dir);
    let db = db_path.to_str().unwrap();

    assert_success(&run_quaid(&home, &["init", db]));
    let (quaid_config_before, legacy_before) = stored_versions(&db_path);

    let output = run_quaid(&home, &["--db", db, "migrate"]);
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("already at schema version 10"),
        "no-op run should say so, got:\n{stdout}"
    );

    assert!(
        !PathBuf::from(format!("{db}.bak")).exists(),
        "no backup should be written for a no-op migration"
    );
    let (quaid_config_after, legacy_after) = stored_versions(&db_path);
    assert_eq!(quaid_config_after, quaid_config_before);
    assert_eq!(legacy_after, legacy_before);
}

#[test]
fn migrate_refuses_database_newer_than_binary() {
    let dir = tempfile::TempDir::new().unwrap();
    let (home, db_path, _vault_root) = temp_layout(&dir);
    let db = db_path.to_str().unwrap();

    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE quaid_config (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL) STRICT;
         CREATE TABLE config (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         INSERT INTO quaid_config (key, value) VALUES
             ('model_id',       'BAAI/bge-small-en-v1.5'),
             ('model_alias',    'small'),
             ('embedding_dim',  '384'),
             ('schema_version', '11');
         INSERT INTO config (key, value) VALUES ('version', '11');",
    )
    .unwrap();
    drop(conn);

    // Positional-path form of the command.
    let output = run_quaid(&home, &["migrate", db]);
    assert!(
        !output.status.success(),
        "future-version databases must be refused\nstdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Found version 11, expected 10"),
        "stderr should name the newer version, got:\n{stderr}"
    );

    assert!(
        !PathBuf::from(format!("{db}.bak")).exists(),
        "no backup should be written when migration is refused"
    );
    let (quaid_config, legacy) = stored_versions(&db_path);
    assert_eq!(quaid_config, "11");
    assert_eq!(legacy, "11");
}
