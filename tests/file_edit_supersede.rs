use std::fs;
use std::path::Path;

use quaid::commands::get::get_page_by_key;
use quaid::core::conversation::file_edit::{
    handle_extracted_edit, is_extracted_path, is_extracted_whitespace_noop,
    is_history_sidecar_path, parse_edited_page, FileEditError, HandleExtractedEditOutcome,
};
use quaid::core::db;
use quaid::core::file_state;
use quaid::mcp::server::{MemoryGraphInput, MemoryPutInput, MemorySearchInput, QuaidServer};
use rusqlite::Connection;
use serde_json::Value;

fn open_test_db(path: &Path) -> Connection {
    db::open(path.to_str().unwrap()).unwrap()
}

fn extract_text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|content| match &content.raw {
            rmcp::model::RawContent::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn put_page(server: &QuaidServer, slug: &str, content: &str) {
    server
        .memory_put(MemoryPutInput {
            slug: slug.to_string(),
            content: content.to_string(),
            expected_version: None,
            namespace: None,
        })
        .unwrap();
}

fn set_default_root(conn: &Connection, root: &Path) {
    fs::create_dir_all(root).unwrap();
    conn.execute(
        "UPDATE collections SET root_path = ?1, writable = 1 WHERE id = 1",
        [root.to_string_lossy().to_string()],
    )
    .unwrap();
}

fn page_id(conn: &Connection, slug: &str) -> i64 {
    conn.query_row(
        "SELECT id FROM pages WHERE collection_id = 1 AND slug = ?1",
        [slug],
        |row| row.get(0),
    )
    .unwrap()
}

fn seed_tracked_file(
    conn: &Connection,
    root: &Path,
    relative_path: &str,
    page_id: i64,
    content: &str,
) {
    let absolute = root.join(relative_path);
    fs::create_dir_all(absolute.parent().unwrap()).unwrap();
    fs::write(&absolute, content).unwrap();
    let stat = file_state::stat_file(&absolute).unwrap();
    let sha = file_state::hash_file(&absolute).unwrap();
    file_state::upsert_file_state(conn, 1, relative_path, page_id, &stat, &sha).unwrap();
}

fn slugs(rows: &[Value]) -> Vec<String> {
    let mut values = rows
        .iter()
        .map(|row| row["slug"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    values.sort();
    values
}

fn preference_markdown(slug: &str, title: &str, body: &str, supersedes: Option<&str>) -> String {
    let mut out = format!("---\nslug: {slug}\ntitle: {title}\ntype: preference\n");
    if let Some(supersedes) = supersedes {
        out.push_str(&format!("supersedes: {supersedes}\n"));
    }
    out.push_str("---\n");
    out.push_str(body);
    out.push('\n');
    out
}

#[test]
fn editing_chained_extracted_preference_preserves_one_linear_chain() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("vault");
    let db_path = dir.path().join("memory.db");
    let server = QuaidServer::new(open_test_db(&db_path));
    let conn = open_test_db(&db_path);
    set_default_root(&conn, &root);

    let seed = preference_markdown(
        "preferences/foo-seed",
        "Foo seed",
        "shared marker seed",
        None,
    );
    let head = preference_markdown(
        "preferences/foo",
        "Foo head",
        "shared marker original head",
        Some("preferences/foo-seed"),
    );
    put_page(&server, "preferences/foo-seed", &seed);
    put_page(&server, "preferences/foo", &head);

    let head_id = page_id(&conn, "preferences/foo");
    seed_tracked_file(&conn, &root, "extracted/preferences/foo.md", head_id, &head);

    let edited = preference_markdown(
        "preferences/foo",
        "Foo head",
        "shared marker edited head",
        Some("preferences/foo-seed"),
    );
    let live_path = root.join("extracted").join("preferences").join("foo.md");
    fs::write(&live_path, &edited).unwrap();
    let stat = file_state::stat_file(&live_path).unwrap();
    let edited_page = parse_edited_page(edited.as_bytes(), &live_path, &root).unwrap();
    let prior_page = get_page_by_key(&conn, 1, "preferences/foo").unwrap();

    let outcome = handle_extracted_edit(
        &conn,
        1,
        head_id,
        Path::new("extracted/preferences/foo.md"),
        &root,
        &stat,
        &prior_page,
        &edited_page,
        edited.as_bytes(),
    )
    .unwrap();

    let archived_slug = match outcome {
        HandleExtractedEditOutcome::Superseded { archived_slug, .. } => archived_slug,
        other => panic!("expected supersede outcome, got {other:?}"),
    };

    let seed_successor: i64 = conn
        .query_row(
            "SELECT superseded_by FROM pages WHERE slug = 'preferences/foo-seed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let archived_id = page_id(&conn, &archived_slug);
    let archived_successor: i64 = conn
        .query_row(
            "SELECT superseded_by FROM pages WHERE slug = ?1",
            [archived_slug.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(seed_successor, archived_id);
    assert_eq!(archived_successor, head_id);

    let head_row: (Option<i64>, String) = conn
        .query_row(
            "SELECT superseded_by, frontmatter FROM pages WHERE id = ?1",
            [head_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert!(head_row.0.is_none());
    let head_frontmatter: Value = serde_json::from_str(&head_row.1).unwrap();
    assert_eq!(head_frontmatter["supersedes"], archived_slug);
    assert_eq!(head_frontmatter["corrected_via"], "file_edit");

    let live_file = fs::read_to_string(&live_path).unwrap();
    assert!(live_file.contains("corrected_via: file_edit"));
    assert!(live_file.contains(&format!("supersedes: {archived_slug}")));

    let search_default: Vec<Value> = serde_json::from_str(&extract_text(
        &server
            .memory_search(MemorySearchInput {
                query: "shared marker".to_string(),
                collection: None,
                namespace: None,
                wing: None,
                limit: None,
                include_superseded: None,
            })
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(
        slugs(&search_default),
        vec!["default::preferences/foo".to_string()]
    );

    let search_history: Vec<Value> = serde_json::from_str(&extract_text(
        &server
            .memory_search(MemorySearchInput {
                query: "shared marker".to_string(),
                collection: None,
                namespace: None,
                wing: None,
                limit: None,
                include_superseded: Some(true),
            })
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(
        slugs(&search_history),
        vec![
            "default::preferences/foo".to_string(),
            format!("default::{archived_slug}"),
            "default::preferences/foo-seed".to_string(),
        ]
    );

    let graph: Value = serde_json::from_str(&extract_text(
        &server
            .memory_graph(MemoryGraphInput {
                slug: "preferences/foo".to_string(),
                depth: Some(3),
                temporal: Some("all".to_string()),
            })
            .unwrap(),
    ))
    .unwrap();
    let supersede_edges = graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|edge| edge["relationship"] == "superseded_by")
        .map(|edge| {
            (
                edge["from"].as_str().unwrap().to_string(),
                edge["to"].as_str().unwrap().to_string(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        supersede_edges,
        vec![
            (
                format!("default::{archived_slug}"),
                "default::preferences/foo".to_string(),
            ),
            (
                "default::preferences/foo-seed".to_string(),
                format!("default::{archived_slug}"),
            ),
        ]
    );
}

#[test]
fn whitespace_only_edit_is_true_noop() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("vault");
    let db_path = dir.path().join("memory.db");
    let server = QuaidServer::new(open_test_db(&db_path));
    let conn = open_test_db(&db_path);
    set_default_root(&conn, &root);

    let original = preference_markdown("preferences/noop", "Noop", "body", None);
    put_page(&server, "preferences/noop", &original);
    let page_id = page_id(&conn, "preferences/noop");
    seed_tracked_file(
        &conn,
        &root,
        "extracted/preferences/noop.md",
        page_id,
        &original,
    );
    let before_version: i64 = conn
        .query_row(
            "SELECT version FROM pages WHERE id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();
    let before_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();
    let before_file_state = file_state::get_file_state(&conn, 1, "extracted/preferences/noop.md")
        .unwrap()
        .unwrap();

    let whitespace_only =
        "---\nslug: preferences/noop\n title: Noop\ntype: preference\n---\nbody  \n\n";
    let live_path = root.join("extracted").join("preferences").join("noop.md");
    fs::write(&live_path, whitespace_only).unwrap();
    let stat = file_state::stat_file(&live_path).unwrap();
    let edited_page = parse_edited_page(whitespace_only.as_bytes(), &live_path, &root).unwrap();
    let prior_page = get_page_by_key(&conn, 1, "preferences/noop").unwrap();

    let outcome = handle_extracted_edit(
        &conn,
        1,
        page_id,
        Path::new("extracted/preferences/noop.md"),
        &root,
        &stat,
        &prior_page,
        &edited_page,
        whitespace_only.as_bytes(),
    )
    .unwrap();

    assert_eq!(outcome, HandleExtractedEditOutcome::WhitespaceNoOp);
    let after_version: i64 = conn
        .query_row(
            "SELECT version FROM pages WHERE id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();
    let after_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM raw_imports WHERE page_id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap();
    let after_file_state = file_state::get_file_state(&conn, 1, "extracted/preferences/noop.md")
        .unwrap()
        .unwrap();

    assert_eq!(before_version, after_version);
    assert_eq!(before_rows, after_rows);
    assert_eq!(before_file_state.sha256, after_file_state.sha256);
    assert_eq!(before_file_state.mtime_ns, after_file_state.mtime_ns);
}

#[test]
fn non_extracted_or_non_fact_pages_bypass_handler() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("vault");
    let db_path = dir.path().join("memory.db");
    let server = QuaidServer::new(open_test_db(&db_path));
    let conn = open_test_db(&db_path);
    set_default_root(&conn, &root);

    let note = "---\nslug: notes/plain\ntitle: Plain\ntype: concept\n---\nplain body\n";
    put_page(&server, "notes/plain", note);
    let page_id = page_id(&conn, "notes/plain");
    seed_tracked_file(&conn, &root, "notes/plain.md", page_id, note);

    let edited = "---\nslug: notes/plain\ntitle: Plain\ntype: concept\n---\nedited body\n";
    let live_path = root.join("notes").join("plain.md");
    fs::write(&live_path, edited).unwrap();
    let stat = file_state::stat_file(&live_path).unwrap();
    let edited_page = parse_edited_page(edited.as_bytes(), &live_path, &root).unwrap();
    let prior_page = get_page_by_key(&conn, 1, "notes/plain").unwrap();

    let outcome = handle_extracted_edit(
        &conn,
        1,
        page_id,
        Path::new("notes/plain.md"),
        &root,
        &stat,
        &prior_page,
        &edited_page,
        edited.as_bytes(),
    )
    .unwrap();

    assert_eq!(outcome, HandleExtractedEditOutcome::Bypass);
    assert!(!is_extracted_path(Path::new("notes/plain.md")));
}

#[test]
fn history_on_disk_writes_sidecar_without_extra_live_page() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("vault");
    let db_path = dir.path().join("memory.db");
    let server = QuaidServer::new(open_test_db(&db_path));
    let conn = open_test_db(&db_path);
    set_default_root(&conn, &root);
    conn.execute(
        "INSERT OR REPLACE INTO config(key, value) VALUES ('corrections.history_on_disk', 'true')",
        [],
    )
    .unwrap();

    let original = preference_markdown("preferences/disk", "Disk", "disk marker original", None);
    put_page(&server, "preferences/disk", &original);
    let page_id = page_id(&conn, "preferences/disk");
    seed_tracked_file(
        &conn,
        &root,
        "extracted/preferences/disk.md",
        page_id,
        &original,
    );

    let edited = preference_markdown("preferences/disk", "Disk", "disk marker edited", None);
    let live_path = root.join("extracted").join("preferences").join("disk.md");
    fs::write(&live_path, &edited).unwrap();
    let stat = file_state::stat_file(&live_path).unwrap();
    let edited_page = parse_edited_page(edited.as_bytes(), &live_path, &root).unwrap();
    let prior_page = get_page_by_key(&conn, 1, "preferences/disk").unwrap();

    let history_path = match handle_extracted_edit(
        &conn,
        1,
        page_id,
        Path::new("extracted/preferences/disk.md"),
        &root,
        &stat,
        &prior_page,
        &edited_page,
        edited.as_bytes(),
    )
    .unwrap()
    {
        HandleExtractedEditOutcome::Superseded {
            history_path: Some(history_path),
            ..
        } => history_path,
        other => panic!("expected disk history sidecar, got {other:?}"),
    };

    let absolute_history_path = root.join(&history_path);
    assert!(absolute_history_path.exists());
    assert!(is_history_sidecar_path(Path::new(&history_path)));
    let page_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE slug = 'preferences/disk'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(page_count, 1);
}

#[test]
fn whitespace_helper_and_non_head_guard_cover_manual_edit_edges() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("vault");
    let db_path = dir.path().join("memory.db");
    let server = QuaidServer::new(open_test_db(&db_path));
    let conn = open_test_db(&db_path);
    set_default_root(&conn, &root);

    let original = preference_markdown("preferences/edge", "Edge", "body", None);
    put_page(&server, "preferences/edge", &original);
    let page_id = page_id(&conn, "preferences/edge");
    seed_tracked_file(
        &conn,
        &root,
        "extracted/preferences/edge.md",
        page_id,
        &original,
    );

    let whitespace_only =
        "---\nslug: preferences/edge\ntitle: Edge\ntype: preference\n---\nbody  \n";
    let live_path = root.join("extracted").join("preferences").join("edge.md");
    fs::write(&live_path, whitespace_only).unwrap();
    assert!(is_extracted_whitespace_noop(
        &conn,
        1,
        &root,
        Path::new("extracted/preferences/edge.md"),
        page_id,
    )
    .unwrap());

    conn.execute(
        "INSERT INTO pages
             (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
         VALUES (1, 'preferences/edge-successor', ?1, 'preference', 'Edge successor', '', 'later', '', '{}', 'preferences', '', 1)",
        [uuid::Uuid::now_v7().to_string()],
    )
    .unwrap();
    let successor_id = conn.last_insert_rowid();
    conn.execute(
        "UPDATE pages SET superseded_by = ?1 WHERE id = ?2",
        rusqlite::params![successor_id, page_id],
    )
    .unwrap();

    let prior_page = get_page_by_key(&conn, 1, "preferences/edge").unwrap();
    let stat = file_state::stat_file(&live_path).unwrap();
    let edited_page = parse_edited_page(whitespace_only.as_bytes(), &live_path, &root).unwrap();
    let error = handle_extracted_edit(
        &conn,
        1,
        page_id,
        Path::new("extracted/preferences/edge.md"),
        &root,
        &stat,
        &prior_page,
        &edited_page,
        whitespace_only.as_bytes(),
    )
    .unwrap_err();

    match error {
        FileEditError::NonHeadTarget { successor_slug, .. } => {
            assert_eq!(successor_slug, "default::preferences/edge-successor");
        }
        other => panic!("expected NonHeadTarget, got {other:?}"),
    }
}
