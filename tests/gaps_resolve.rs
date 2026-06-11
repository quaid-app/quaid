#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Knowledge-gap resolve surface and context-persistence tests
//! (fable-review area 14, items 3-4).
//!
//! Scenarios tested:
//!   1. `quaid gaps resolve <id> <slug>` flips `resolved_at` /
//!      `resolved_by_slug`; unknown ids and slugs fail
//!   2. the `memory_gap_resolve` MCP tool does the same and 404s unknown
//!      ids; `quaid call` dispatch routes it (registry/dispatch parity)
//!   3. `memory_gap` persists caller context only when the
//!      `gaps.store_context` config key is `true`

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;
#[path = "common/mcp_harness.rs"]
mod harness;

use std::path::{Path, PathBuf};
use std::process::Command;

use harness::{create_page, extract_text};
use quaid::commands::call::dispatch_tool;
use quaid::core::{db, gaps};
use quaid::mcp::server::{MemoryGapInput, MemoryGapResolveInput, MemoryGapsInput, QuaidServer};
use rusqlite::Connection;
use serde_json::json;

fn open_test_db(path: &Path) -> Connection {
    db::open(path.to_str().expect("utf-8 db path")).expect("open test db")
}

fn insert_page(conn: &Connection, slug: &str) {
    conn.execute(
        "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES (?1, 'concept', ?1, '', '', '', '{}', 'notes', '', 1)",
        rusqlite::params![slug],
    )
    .expect("insert page");
}

fn run_quaid(db_path: &Path, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    command.arg("--db").arg(db_path).args(args);
    command.output().expect("run quaid")
}

fn test_db_path(dir: &tempfile::TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

fn gap_row(conn: &Connection, id: i64) -> (Option<String>, Option<String>) {
    conn.query_row(
        "SELECT resolved_at, resolved_by_slug FROM knowledge_gaps WHERE id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .expect("gap row")
}

// ── 1. CLI resolve ────────────────────────────────────────────────────────────

#[test]
fn cli_gaps_resolve_flips_resolved_fields() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "gaps-resolve-cli.db");
    let conn = open_test_db(&db_path);
    insert_page(&conn, "notes/answer");
    gaps::log_gap(None, "what is the answer", "", Some(0.1), &conn).expect("log gap");
    let gap_id: i64 = conn
        .query_row("SELECT id FROM knowledge_gaps LIMIT 1", [], |r| r.get(0))
        .unwrap();
    drop(conn);

    let output = run_quaid(
        &db_path,
        &["gaps", "resolve", &gap_id.to_string(), "notes/answer"],
    );
    assert!(output.status.success(), "resolve must succeed: {output:?}");

    let conn = open_test_db(&db_path);
    let (resolved_at, resolved_by_slug) = gap_row(&conn, gap_id);
    assert!(resolved_at.is_some(), "resolved_at must be set");
    assert_eq!(resolved_by_slug.as_deref(), Some("notes/answer"));
    drop(conn);

    // The resolved listing surfaces the resolution slug.
    let listing = run_quaid(&db_path, &["--json", "gaps", "--resolved"]);
    assert!(listing.status.success(), "{listing:?}");
    let entries: serde_json::Value = serde_json::from_slice(&listing.stdout).unwrap();
    assert_eq!(
        entries.as_array().unwrap()[0]["resolved_by_slug"],
        json!("notes/answer")
    );
}

#[test]
fn cli_gaps_resolve_unknown_id_fails() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "gaps-resolve-cli-404.db");
    let conn = open_test_db(&db_path);
    insert_page(&conn, "notes/answer");
    drop(conn);

    let output = run_quaid(&db_path, &["gaps", "resolve", "9999", "notes/answer"]);

    assert!(!output.status.success(), "unknown gap id must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("gap not found"),
        "stderr should report the missing gap: {stderr}"
    );
}

#[test]
fn cli_gaps_resolve_unknown_slug_fails() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "gaps-resolve-cli-noslug.db");
    let conn = open_test_db(&db_path);
    gaps::log_gap(None, "dangling question", "", None, &conn).expect("log gap");
    let gap_id: i64 = conn
        .query_row("SELECT id FROM knowledge_gaps LIMIT 1", [], |r| r.get(0))
        .unwrap();
    drop(conn);

    let output = run_quaid(
        &db_path,
        &["gaps", "resolve", &gap_id.to_string(), "notes/ghost"],
    );

    assert!(!output.status.success(), "unknown slug must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("page not found"),
        "stderr should report the missing page: {stderr}"
    );

    let conn = open_test_db(&db_path);
    let (resolved_at, _) = gap_row(&conn, gap_id);
    assert!(resolved_at.is_none(), "gap must stay unresolved");
}

// ── 2. MCP resolve ────────────────────────────────────────────────────────────

#[test]
fn memory_gap_resolve_flips_resolved_fields() {
    let (_dir, conn) = harness::open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "notes/answer",
        "---\ntitle: Answer\ntype: concept\n---\nthe answer\n",
    );
    let logged = server
        .memory_gap(MemoryGapInput {
            query: "what is the answer".to_string(),
            slug: None,
            context: None,
        })
        .unwrap();
    let logged_json: serde_json::Value = serde_json::from_str(&extract_text(&logged)).unwrap();
    let gap_id = logged_json["id"].as_i64().unwrap();

    let resolved = server
        .memory_gap_resolve(MemoryGapResolveInput {
            id: gap_id,
            slug: "notes/answer".to_string(),
        })
        .unwrap();
    let resolved_json: serde_json::Value = serde_json::from_str(&extract_text(&resolved)).unwrap();
    assert_eq!(resolved_json["id"], json!(gap_id));
    assert_eq!(resolved_json["resolved_by_slug"], json!("notes/answer"));

    let listing = server
        .memory_gaps(MemoryGapsInput {
            resolved: Some(true),
            limit: None,
        })
        .unwrap();
    let entries: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&listing)).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0]["resolved_at"].is_string());
    assert_eq!(entries[0]["resolved_by_slug"], json!("notes/answer"));
}

#[test]
fn memory_gap_resolve_unknown_id_returns_not_found() {
    let (_dir, conn) = harness::open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "notes/answer",
        "---\ntitle: Answer\ntype: concept\n---\nthe answer\n",
    );

    let err = server
        .memory_gap_resolve(MemoryGapResolveInput {
            id: 9999,
            slug: "notes/answer".to_string(),
        })
        .expect_err("unknown gap id must error");
    assert!(
        err.message.contains("gap not found"),
        "error must report the missing gap: {}",
        err.message
    );
}

#[test]
fn memory_gap_resolve_unknown_slug_returns_not_found() {
    let (_dir, conn) = harness::open_test_db();
    let server = QuaidServer::new(conn);

    let err = server
        .memory_gap_resolve(MemoryGapResolveInput {
            id: 1,
            slug: "notes/ghost".to_string(),
        })
        .expect_err("unknown slug must error");
    assert!(
        err.message.contains("not found"),
        "error must report the missing page: {}",
        err.message
    );
}

#[test]
fn call_dispatch_routes_memory_gap_resolve() {
    let (_dir, conn) = harness::open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "notes/answer",
        "---\ntitle: Answer\ntype: concept\n---\nthe answer\n",
    );
    let logged = dispatch_tool(&server, "memory_gap", json!({"query": "dispatch question"}))
        .expect("dispatch memory_gap");
    let gap_id = logged["id"].as_i64().unwrap();

    let resolved = dispatch_tool(
        &server,
        "memory_gap_resolve",
        json!({"id": gap_id, "slug": "notes/answer"}),
    )
    .expect("dispatch memory_gap_resolve");

    assert_eq!(resolved["resolved_by_slug"], json!("notes/answer"));
}

// ── 3. context persistence gating ─────────────────────────────────────────────

fn logged_contexts(server: &QuaidServer) -> Vec<String> {
    let listing = server
        .memory_gaps(MemoryGapsInput {
            resolved: None,
            limit: None,
        })
        .unwrap();
    serde_json::from_str::<Vec<serde_json::Value>>(&extract_text(&listing))
        .unwrap()
        .into_iter()
        .map(|entry| entry["context"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn memory_gap_discards_context_by_default() {
    let (_dir, conn) = harness::open_test_db();
    let seeded: String = conn
        .query_row(
            "SELECT value FROM config WHERE key = 'gaps.store_context'",
            [],
            |row| row.get(0),
        )
        .expect("gaps.store_context must be seeded");
    assert_eq!(seeded, "false", "schema must seed default-discard");
    let server = QuaidServer::new(conn);

    server
        .memory_gap(MemoryGapInput {
            query: "question one".to_string(),
            slug: None,
            context: Some("caller-provided context".to_string()),
        })
        .unwrap();

    assert_eq!(
        logged_contexts(&server),
        vec![String::new()],
        "default posture discards caller context"
    );
}

#[test]
fn memory_gap_persists_context_when_store_context_enabled() {
    let (_dir, conn) = harness::open_test_db();
    conn.execute(
        "UPDATE config SET value = 'true' WHERE key = 'gaps.store_context'",
        [],
    )
    .unwrap();
    let server = QuaidServer::new(conn);

    server
        .memory_gap(MemoryGapInput {
            query: "question two".to_string(),
            slug: None,
            context: Some("caller-provided context".to_string()),
        })
        .unwrap();

    assert_eq!(
        logged_contexts(&server),
        vec!["caller-provided context".to_owned()],
        "opted-in gap must persist its (length-capped) context"
    );
}
