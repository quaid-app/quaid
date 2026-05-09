#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for collection-aware slug routing across the read/write
//! CLI surface (`get`, `query`, `put`, `link`, `unlink`, `links`, `backlinks`,
//! `graph`, `timeline`, `check`, `list`, `search`).

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;
#[path = "common/truth_fixtures.rs"]
mod truth_fixtures;

use truth_fixtures::*;

#[test]
fn put_cli_refuses_when_collection_is_persisted_read_only() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "put-read-only.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    conn.execute(
        "UPDATE collections SET writable = 0 WHERE id = ?1",
        [collection_id],
    )
    .expect("mark collection read-only");
    drop(conn);

    let output = run_quaid_with_stdin(
        &db_path,
        &["put", "work::notes/read-only"],
        "---\ntitle: Read Only\ntype: note\n---\nhello\n",
    );

    assert!(
        !output.status.success(),
        "put should fail for read-only collection: {output:?}"
    );
    #[cfg(unix)]
    assert!(
        combined_output(&output).contains("CollectionReadOnlyError"),
        "put must surface CollectionReadOnlyError: {output:?}"
    );
    #[cfg(not(unix))]
    assert!(
        combined_output(&output).contains("UnsupportedPlatformError"),
        "Windows put must fail closed with UnsupportedPlatformError: {output:?}"
    );
}

#[test]
fn cli_get_accepts_explicit_collection_slug_and_rejects_ambiguous_bare_slug() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cli-get-parity.db");
    let conn = open_test_db(&db_path);
    let work_root = dir.path().join("work");
    let memory_root = dir.path().join("memory");
    std::fs::create_dir_all(&work_root).expect("create work root");
    std::fs::create_dir_all(&memory_root).expect("create memory root");
    let work_id = insert_collection(&conn, "work", &work_root);
    let memory_id = insert_collection(&conn, "memory", &memory_root);
    insert_page(&conn, work_id, "notes/meeting");
    insert_page(&conn, memory_id, "notes/meeting");
    drop(conn);

    let ambiguous = run_quaid(&db_path, &["get", "notes/meeting"]);
    assert_ambiguous_slug_failure(
        &ambiguous,
        "notes/meeting",
        &["work::notes/meeting", "memory::notes/meeting"],
    );

    let explicit = run_quaid(&db_path, &["--json", "get", "work::notes/meeting"]);
    assert!(
        explicit.status.success(),
        "explicit collection slug should succeed: {explicit:?}"
    );
    let parsed = parse_stdout_json(&explicit);
    assert_eq!(parsed["slug"].as_str(), Some("work::notes/meeting"));
    assert_eq!(
        parsed["frontmatter"]["slug"].as_str(),
        Some("work::notes/meeting")
    );
}

#[test]
fn cli_query_rejects_ambiguous_exact_slug_input() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cli-query-ambiguous.db");
    let conn = open_test_db(&db_path);
    let work_root = dir.path().join("work");
    let memory_root = dir.path().join("memory");
    std::fs::create_dir_all(&work_root).expect("create work root");
    std::fs::create_dir_all(&memory_root).expect("create memory root");
    let work_id = insert_collection(&conn, "work", &work_root);
    let memory_id = insert_collection(&conn, "memory", &memory_root);
    insert_page_with_truth(&conn, work_id, "notes/meeting", "work note");
    insert_page_with_truth(&conn, memory_id, "notes/meeting", "memory note");
    drop(conn);

    let bare = run_quaid(&db_path, &["query", "notes/meeting"]);
    assert_ambiguous_slug_failure(
        &bare,
        "notes/meeting",
        &["work::notes/meeting", "memory::notes/meeting"],
    );

    let bracketed = run_quaid(&db_path, &["query", "[[notes/meeting]]"]);
    assert_ambiguous_slug_failure(
        &bracketed,
        "notes/meeting",
        &["work::notes/meeting", "memory::notes/meeting"],
    );

    let explicit = run_quaid(&db_path, &["--json", "query", "work::notes/meeting"]);
    assert!(
        explicit.status.success(),
        "explicit collection slug should route query successfully: {explicit:?}"
    );
    let parsed = parse_stdout_json(&explicit);
    assert_eq!(parsed[0]["slug"].as_str(), Some("work::notes/meeting"));
}

#[test]
fn cli_read_slug_commands_reject_ambiguous_bare_slugs() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cli-read-ambiguous.db");
    let conn = open_test_db(&db_path);
    let work_root = dir.path().join("work");
    let memory_root = dir.path().join("memory");
    std::fs::create_dir_all(&work_root).expect("create work root");
    std::fs::create_dir_all(&memory_root).expect("create memory root");
    let work_id = insert_collection(&conn, "work", &work_root);
    let memory_id = insert_collection(&conn, "memory", &memory_root);
    insert_page(&conn, work_id, "notes/shared");
    insert_page(&conn, memory_id, "notes/shared");
    drop(conn);

    let candidates = ["work::notes/shared", "memory::notes/shared"];

    let graph = run_quaid(&db_path, &["graph", "notes/shared", "--depth", "1"]);
    assert_ambiguous_slug_failure(&graph, "notes/shared", &candidates);

    let timeline = run_quaid(&db_path, &["timeline", "notes/shared"]);
    assert_ambiguous_slug_failure(&timeline, "notes/shared", &candidates);

    let links = run_quaid(&db_path, &["links", "notes/shared"]);
    assert_ambiguous_slug_failure(&links, "notes/shared", &candidates);

    let backlinks = run_quaid(&db_path, &["backlinks", "notes/shared"]);
    assert_ambiguous_slug_failure(&backlinks, "notes/shared", &candidates);
}

#[test]
fn cli_write_slug_commands_reject_ambiguous_bare_slugs() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cli-write-ambiguous.db");
    let conn = open_test_db(&db_path);
    let work_root = dir.path().join("work");
    let memory_root = dir.path().join("memory");
    std::fs::create_dir_all(&work_root).expect("create work root");
    std::fs::create_dir_all(&memory_root).expect("create memory root");
    let work_id = insert_collection(&conn, "work", &work_root);
    let memory_id = insert_collection(&conn, "memory", &memory_root);
    insert_page_with_truth(
        &conn,
        work_id,
        "notes/shared",
        "## Assertions\nAlice works at Acme.\n",
    );
    insert_page_with_truth(
        &conn,
        memory_id,
        "notes/shared",
        "## Assertions\nAlice works at Beta.\n",
    );
    insert_page(&conn, work_id, "notes/target");
    drop(conn);

    let candidates = ["work::notes/shared", "memory::notes/shared"];

    let check = run_quaid(&db_path, &["check", "notes/shared"]);
    assert_ambiguous_slug_failure(&check, "notes/shared", &candidates);

    let link = run_quaid(
        &db_path,
        &[
            "link",
            "notes/shared",
            "work::notes/target",
            "--relationship",
            "relates",
        ],
    );
    assert_ambiguous_slug_failure(&link, "notes/shared", &candidates);

    let unlink = run_quaid(&db_path, &["unlink", "notes/shared", "work::notes/target"]);
    assert_ambiguous_slug_failure(&unlink, "notes/shared", &candidates);
}

#[test]
fn cli_unlink_no_match_reports_canonical_resolved_addresses() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cli-unlink-canonical.db");
    let conn = open_test_db(&db_path);
    let work_root = dir.path().join("work");
    let memory_root = dir.path().join("memory");
    std::fs::create_dir_all(&work_root).expect("create work root");
    std::fs::create_dir_all(&memory_root).expect("create memory root");
    let work_id = insert_collection(&conn, "work", &work_root);
    let memory_id = insert_collection(&conn, "memory", &memory_root);
    insert_page(&conn, work_id, "notes/a");
    insert_page(&conn, memory_id, "notes/b");
    drop(conn);

    let output = run_quaid(&db_path, &["unlink", "notes/a", "notes/b"]);
    assert!(
        !output.status.success(),
        "unlink should fail when no matching link exists: {output:?}"
    );
    let text = combined_output(&output);
    assert!(
        text.contains("no matching link found between work::notes/a and memory::notes/b"),
        "unlink should report canonical resolved addresses on the no-match path: {text}"
    );
}

#[test]
fn cli_unlink_accepts_explicit_collection_slugs() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cli-unlink-explicit.db");
    let conn = open_test_db(&db_path);
    let work_root = dir.path().join("work");
    let memory_root = dir.path().join("memory");
    std::fs::create_dir_all(&work_root).expect("create work root");
    std::fs::create_dir_all(&memory_root).expect("create memory root");
    let work_id = insert_collection(&conn, "work", &work_root);
    let memory_id = insert_collection(&conn, "memory", &memory_root);
    insert_page(&conn, work_id, "notes/a");
    insert_page(&conn, memory_id, "notes/a");
    insert_page(&conn, work_id, "notes/b");
    insert_page(&conn, memory_id, "notes/b");
    drop(conn);

    let link = run_quaid(
        &db_path,
        &[
            "link",
            "work::notes/a",
            "memory::notes/b",
            "--relationship",
            "relates",
        ],
    );
    assert!(link.status.success(), "setup link should succeed: {link:?}");

    let unlink = run_quaid(
        &db_path,
        &[
            "unlink",
            "work::notes/a",
            "memory::notes/b",
            "--relationship",
            "relates",
        ],
    );
    assert!(
        unlink.status.success(),
        "explicit collection slug should route unlink successfully: {unlink:?}"
    );
    let text = String::from_utf8_lossy(&unlink.stdout);
    assert!(
        text.contains("Removed 1 link(s) work::notes/a → memory::notes/b"),
        "unlink should report canonical explicit addresses: {text}"
    );

    let conn = open_test_db(&db_path);
    let remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM links l
             JOIN pages fp ON fp.id = l.from_page_id
             JOIN pages tp ON tp.id = l.to_page_id
             JOIN collections fc ON fc.id = fp.collection_id
             JOIN collections tc ON tc.id = tp.collection_id
             WHERE fc.name = 'work'
               AND fp.slug = 'notes/a'
               AND tc.name = 'memory'
               AND tp.slug = 'notes/b'
               AND l.relationship = 'relates'",
            [],
            |row| row.get(0),
        )
        .expect("count remaining explicit link");
    assert_eq!(remaining, 0);
}

#[test]
fn cli_link_views_and_graph_emit_canonical_page_addresses() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cli-link-graph-parity.db");
    let conn = open_test_db(&db_path);
    let work_root = dir.path().join("work");
    let memory_root = dir.path().join("memory");
    std::fs::create_dir_all(&work_root).expect("create work root");
    std::fs::create_dir_all(&memory_root).expect("create memory root");
    let work_id = insert_collection(&conn, "work", &work_root);
    let memory_id = insert_collection(&conn, "memory", &memory_root);
    insert_page(&conn, work_id, "notes/a");
    insert_page(&conn, memory_id, "notes/b");
    drop(conn);

    let link_output = run_quaid(
        &db_path,
        &[
            "link",
            "work::notes/a",
            "memory::notes/b",
            "--relationship",
            "relates",
        ],
    );
    assert!(
        link_output.status.success(),
        "link should succeed: {link_output:?}"
    );
    let link_text = String::from_utf8_lossy(&link_output.stdout);
    assert!(link_text.contains("work::notes/a"));
    assert!(link_text.contains("memory::notes/b"));

    let outbound = run_quaid(&db_path, &["--json", "links", "work::notes/a"]);
    assert!(
        outbound.status.success(),
        "links should succeed: {outbound:?}"
    );
    let outbound_json = parse_stdout_json(&outbound);
    assert_eq!(
        outbound_json[0]["to_slug"].as_str(),
        Some("memory::notes/b")
    );

    let inbound = run_quaid(&db_path, &["--json", "backlinks", "memory::notes/b"]);
    assert!(
        inbound.status.success(),
        "backlinks should succeed: {inbound:?}"
    );
    let inbound_json = parse_stdout_json(&inbound);
    assert_eq!(inbound_json[0]["from_slug"].as_str(), Some("work::notes/a"));

    let graph = run_quaid(
        &db_path,
        &["--json", "graph", "work::notes/a", "--depth", "1"],
    );
    assert!(graph.status.success(), "graph should succeed: {graph:?}");
    let graph_json = parse_stdout_json(&graph);
    let node_slugs: Vec<_> = graph_json["nodes"]
        .as_array()
        .expect("graph nodes")
        .iter()
        .map(|node| node["slug"].as_str().expect("node slug"))
        .collect();
    assert!(node_slugs.contains(&"work::notes/a"));
    assert!(node_slugs.contains(&"memory::notes/b"));
    assert_eq!(
        graph_json["edges"][0]["from"].as_str(),
        Some("work::notes/a")
    );
    assert_eq!(
        graph_json["edges"][0]["to"].as_str(),
        Some("memory::notes/b")
    );
}

#[test]
fn cli_timeline_and_check_emit_canonical_slugs_for_explicit_routes() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cli-timeline-check-parity.db");
    let conn = open_test_db(&db_path);
    let work_root = dir.path().join("work");
    std::fs::create_dir_all(&work_root).expect("create work root");
    let work_id = insert_collection(&conn, "work", &work_root);
    insert_page_with_truth(
        &conn,
        work_id,
        "people/alice",
        "## Assertions\nAlice works at Acme Corp.\n",
    );
    insert_page_with_truth(
        &conn,
        work_id,
        "sources/alice-profile",
        "## Assertions\nAlice works at Beta Corp.\n",
    );
    insert_timeline_entry(
        &conn,
        page_id(&conn, work_id, "people/alice"),
        "2026-04-24",
        "joined",
    );
    drop(conn);

    let timeline = run_quaid(&db_path, &["--json", "timeline", "work::people/alice"]);
    assert!(
        timeline.status.success(),
        "timeline should succeed for explicit slug: {timeline:?}"
    );
    let timeline_json = parse_stdout_json(&timeline);
    assert_eq!(timeline_json["slug"].as_str(), Some("work::people/alice"));

    let warmup = run_quaid(&db_path, &["check", "--all"]);
    assert!(
        warmup.status.success(),
        "all-mode check should seed contradiction rows: {warmup:?}"
    );

    let check = run_quaid(&db_path, &["--json", "check", "work::people/alice"]);
    assert!(
        check.status.success(),
        "check should succeed for explicit slug: {check:?}"
    );
    let check_json = parse_stdout_json(&check);
    assert_eq!(
        check_json[0]["page_slug"].as_str(),
        Some("work::people/alice")
    );
    assert_eq!(
        check_json[0]["other_page_slug"].as_str(),
        Some("work::sources/alice-profile")
    );
}

#[test]
fn cli_list_search_and_query_emit_canonical_slugs() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "cli-search-query-list-parity.db");
    let conn = open_test_db(&db_path);
    let work_root = dir.path().join("work");
    let memory_root = dir.path().join("memory");
    std::fs::create_dir_all(&work_root).expect("create work root");
    std::fs::create_dir_all(&memory_root).expect("create memory root");
    let work_id = insert_collection(&conn, "work", &work_root);
    let memory_id = insert_collection(&conn, "memory", &memory_root);
    insert_page_with_truth(
        &conn,
        work_id,
        "people/alice",
        "Alice is the founder of Acme.\n",
    );
    insert_page_with_truth(
        &conn,
        memory_id,
        "people/bob",
        "Bob works on distributed systems.\n",
    );
    drop(conn);

    let list = run_quaid(&db_path, &["--json", "list"]);
    assert!(list.status.success(), "list should succeed: {list:?}");
    let list_json = parse_stdout_json(&list);
    let list_slugs: Vec<_> = list_json
        .as_array()
        .expect("list rows")
        .iter()
        .map(|row| row["slug"].as_str().expect("list slug"))
        .collect();
    assert!(list_slugs.contains(&"work::people/alice"));
    assert!(list_slugs.contains(&"memory::people/bob"));

    let search = run_quaid(&db_path, &["--json", "search", "founder"]);
    assert!(search.status.success(), "search should succeed: {search:?}");
    let search_json = parse_stdout_json(&search);
    assert_eq!(search_json[0]["slug"].as_str(), Some("work::people/alice"));

    let query = run_quaid(&db_path, &["--json", "query", "people/alice"]);
    assert!(query.status.success(), "query should succeed: {query:?}");
    let query_json = parse_stdout_json(&query);
    assert_eq!(query_json[0]["slug"].as_str(), Some("work::people/alice"));
}
