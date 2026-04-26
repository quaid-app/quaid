//! Integration tests for `quaid graph` (tasks 2.5).

use quaid::core::db;
use quaid::core::graph::{self, TemporalFilter};
use rusqlite::Connection;
use std::path::Path;

fn open_test_db(path: &Path) -> Connection {
    db::open(path.to_str().unwrap()).unwrap()
}

fn insert_page(conn: &Connection, slug: &str, page_type: &str, title: &str) {
    conn.execute(
        "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES (?1, ?2, ?3, '', '', '', '{}', '', '', 1)",
        rusqlite::params![slug, page_type, title],
    )
    .unwrap();
}

fn insert_link(
    conn: &Connection,
    from: &str,
    to: &str,
    rel: &str,
    valid_from: Option<&str>,
    valid_until: Option<&str>,
) {
    let from_id: i64 = conn
        .query_row("SELECT id FROM pages WHERE slug = ?1", [from], |row| {
            row.get(0)
        })
        .unwrap();
    let to_id: i64 = conn
        .query_row("SELECT id FROM pages WHERE slug = ?1", [to], |row| {
            row.get(0)
        })
        .unwrap();
    conn.execute(
        "INSERT INTO links (
            from_page_id, to_page_id, relationship, source_kind, valid_from, valid_until
         ) VALUES (?1, ?2, ?3, 'programmatic', ?4, ?5)",
        rusqlite::params![from_id, to_id, rel, valid_from, valid_until],
    )
    .unwrap();
}

// ── Human-readable output format ─────────────────────────────

#[test]
fn graph_cli_human_output_nests_depth_two_edges_under_their_parent() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = open_test_db(&db_path);

    insert_page(&conn, "people/alice", "person", "Alice");
    insert_page(&conn, "companies/acme", "company", "Acme");
    insert_page(&conn, "projects/rocket", "project", "Rocket");
    insert_link(
        &conn,
        "people/alice",
        "companies/acme",
        "works_at",
        None,
        None,
    );
    insert_link(
        &conn,
        "companies/acme",
        "projects/rocket",
        "owns",
        None,
        None,
    );

    let mut out = Vec::<u8>::new();
    quaid::commands::graph::run_to(&conn, "people/alice", 2, "current", false, &mut out).unwrap();
    let output = String::from_utf8(out).unwrap();
    let lines: Vec<_> = output.lines().collect();

    assert_eq!(
        lines,
        vec![
            "default::people/alice",
            "  → default::companies/acme (works_at)",
            "    → default::projects/rocket (owns)",
        ],
        "text output must render depth-2 edges under their parent; got: {output}"
    );
}

#[test]
fn graph_cli_human_output_skips_self_link_back_to_root() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = open_test_db(&db_path);

    insert_page(&conn, "people/alice", "person", "Alice");
    insert_link(&conn, "people/alice", "people/alice", "knows", None, None);

    let mut out = Vec::<u8>::new();
    quaid::commands::graph::run_to(&conn, "people/alice", 1, "current", false, &mut out).unwrap();
    let output = String::from_utf8(out).unwrap();

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["default::people/alice"],
        "self-links must not render the root back into the tree; got: {output}"
    );
    assert!(
        !output.contains("→ default::people/alice"),
        "text output must never contain '→ <root>' for a self-link; got: {output}"
    );
}

#[test]
fn graph_cli_human_self_link_plus_real_neighbour_renders_only_neighbour() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = open_test_db(&db_path);

    insert_page(&conn, "people/alice", "person", "Alice");
    insert_page(&conn, "companies/acme", "company", "Acme");
    insert_link(&conn, "people/alice", "people/alice", "knows", None, None);
    insert_link(
        &conn,
        "people/alice",
        "companies/acme",
        "works_at",
        None,
        None,
    );

    let mut out = Vec::<u8>::new();
    quaid::commands::graph::run_to(&conn, "people/alice", 1, "current", false, &mut out).unwrap();
    let output = String::from_utf8(out).unwrap();
    let lines: Vec<_> = output.lines().collect();

    assert_eq!(
        lines,
        vec![
            "default::people/alice",
            "  → default::companies/acme (works_at)",
        ],
        "self-link must be suppressed but real neighbours must still render; got: {output}"
    );
    assert!(
        !output.contains("→ default::people/alice"),
        "root must never appear as its own neighbour; got: {output}"
    );
}

#[test]
fn graph_cli_human_output_skips_cycle_back_to_root() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = open_test_db(&db_path);

    insert_page(&conn, "a", "concept", "A");
    insert_page(&conn, "b", "concept", "B");
    insert_link(&conn, "a", "b", "related", None, None);
    insert_link(&conn, "b", "a", "related", None, None);

    let mut out = Vec::<u8>::new();
    quaid::commands::graph::run_to(&conn, "a", 2, "all", false, &mut out).unwrap();
    let output = String::from_utf8(out).unwrap();

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["default::a", "  → default::b (related)"],
        "cycles must not render an already-on-path node back into the tree; got: {output}"
    );
}

// ── JSON output is valid JSON with nodes and edges keys ──────

#[test]
fn graph_cli_json_output_has_nodes_and_edges() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = open_test_db(&db_path);

    insert_page(&conn, "people/alice", "person", "Alice");
    insert_page(&conn, "companies/acme", "company", "Acme");
    insert_link(
        &conn,
        "people/alice",
        "companies/acme",
        "works_at",
        None,
        None,
    );

    let mut out = Vec::<u8>::new();
    quaid::commands::graph::run_to(&conn, "people/alice", 2, "current", true, &mut out).unwrap();
    let output = String::from_utf8(out).unwrap();

    let parsed: serde_json::Value =
        serde_json::from_str(output.trim()).expect("CLI --json output must be valid JSON");

    assert!(parsed.get("nodes").is_some(), "JSON must have 'nodes' key");
    assert!(parsed.get("edges").is_some(), "JSON must have 'edges' key");
    assert!(parsed["nodes"].is_array(), "'nodes' must be an array");
    assert!(parsed["edges"].is_array(), "'edges' must be an array");
    assert_eq!(
        parsed["nodes"].as_array().unwrap().len(),
        2,
        "expected 2 nodes"
    );
    assert_eq!(
        parsed["edges"].as_array().unwrap().len(),
        1,
        "expected 1 edge"
    );

    // Verify node output shape: each node has slug, type, title
    let alice = parsed["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["slug"] == "default::people/alice")
        .expect("alice node must be present");
    assert!(alice.get("slug").is_some());
    assert!(alice.get("type").is_some());
    assert!(alice.get("title").is_some());

    // Verify edge output shape: from, to, relationship
    let edge = &parsed["edges"][0];
    assert_eq!(edge["from"], "default::people/alice");
    assert_eq!(edge["to"], "default::companies/acme");
    assert_eq!(edge["relationship"], "works_at");
}

// ── Unknown slug returns error ───────────────────────────────

#[test]
fn graph_cli_unknown_slug_returns_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = open_test_db(&db_path);

    let mut out = Vec::<u8>::new();
    let result =
        quaid::commands::graph::run_to(&conn, "nobody/ghost", 1, "current", false, &mut out);
    assert!(result.is_err());
    assert!(
        result.unwrap_err().to_string().contains("page not found"),
        "error must contain 'page not found'"
    );
}

// ── JSON node objects have expected fields ────────────────────

#[test]
fn graph_json_nodes_have_slug_type_title() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = open_test_db(&db_path);

    insert_page(&conn, "people/alice", "person", "Alice Johnson");

    let result =
        graph::neighborhood_graph("people/alice", 0, TemporalFilter::Active, &conn).unwrap();
    let json_str = serde_json::to_string(&result).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    let node = &parsed["nodes"][0];
    assert_eq!(node["slug"], "people/alice");
    assert_eq!(node["type"], "person");
    assert_eq!(node["title"], "Alice Johnson");
}

// ── JSON edge objects have expected fields ────────────────────

#[test]
fn graph_json_edges_have_from_to_relationship() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = open_test_db(&db_path);

    insert_page(&conn, "people/alice", "person", "Alice");
    insert_page(&conn, "companies/acme", "company", "Acme");
    insert_link(
        &conn,
        "people/alice",
        "companies/acme",
        "works_at",
        Some("2024-01"),
        None,
    );

    let result =
        graph::neighborhood_graph("people/alice", 1, TemporalFilter::Active, &conn).unwrap();
    let json_str = serde_json::to_string(&result).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    let edge = &parsed["edges"][0];
    assert_eq!(edge["from"], "people/alice");
    assert_eq!(edge["to"], "companies/acme");
    assert_eq!(edge["relationship"], "works_at");
    assert_eq!(edge["valid_from"], "2024-01");
    assert!(edge["valid_until"].is_null());
}

// ── Temporal filter mapping in CLI ───────────────────────────

#[test]
fn graph_core_temporal_filter_active_excludes_closed_links() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("test.db");
    let conn = open_test_db(&db_path);

    insert_page(&conn, "people/alice", "person", "Alice");
    insert_page(&conn, "companies/acme", "company", "Acme");
    insert_link(
        &conn,
        "people/alice",
        "companies/acme",
        "works_at",
        Some("2020-01-01"),
        Some("2020-12-31"),
    );

    // "current" (default) should exclude closed link
    let result_current =
        graph::neighborhood_graph("people/alice", 1, TemporalFilter::Active, &conn).unwrap();
    assert_eq!(result_current.nodes.len(), 1);

    // "all" should include closed link
    let result_all =
        graph::neighborhood_graph("people/alice", 1, TemporalFilter::All, &conn).unwrap();
    assert_eq!(result_all.nodes.len(), 2);
}
