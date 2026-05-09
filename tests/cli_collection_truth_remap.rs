#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]
#![cfg(unix)]

//! Integration tests for `quaid collection sync --remap-root` truth-merge behavior.

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;
#[path = "common/truth_fixtures.rs"]
mod truth_fixtures;

use rusqlite::params;
use truth_fixtures::*;

#[cfg(unix)]
#[test]
fn offline_remap_completes_inline_and_preserves_page_identity() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "offline-remap-cli.db");
    let conn = open_test_db(&db_path);
    let source_root = dir.path().join("source");
    let target_root = dir.path().join("remapped");
    std::fs::create_dir_all(source_root.join("notes")).expect("create source root");
    let collection_id = insert_collection(&conn, "work", &source_root);
    let raw_bytes =
        b"---\nmemory_id: 11111111-1111-7111-8111-111111111111\nquaid_id: 11111111-1111-7111-8111-111111111111\nslug: notes/a\ntitle: Remapped Note\ntype: concept\n---\nhello from remap\n";
    let sibling_bytes =
        b"---\nmemory_id: 22222222-2222-7222-8222-222222222222\nquaid_id: 22222222-2222-7222-8222-222222222222\nslug: notes/b\ntitle: Sibling Note\ntype: concept\n---\nhello from sibling\n";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/a",
        "11111111-1111-7111-8111-111111111111",
        raw_bytes,
        "notes/old-a.md",
    );
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/b",
        "22222222-2222-7222-8222-222222222222",
        sibling_bytes,
        "notes/b.md",
    );
    let page_a = page_id(&conn, collection_id, "notes/a");
    let page_b = page_id(&conn, collection_id, "notes/b");
    insert_programmatic_link(&conn, page_a, page_b);
    std::fs::write(source_root.join("notes").join("old-a.md"), raw_bytes)
        .expect("seed source note");
    std::fs::write(source_root.join("notes").join("b.md"), sibling_bytes)
        .expect("seed source sibling");
    drop(conn);

    std::fs::create_dir_all(target_root.join("nested")).expect("create nested dir");
    std::fs::create_dir_all(target_root.join("notes")).expect("create notes dir");
    std::fs::write(target_root.join("nested").join("renamed-a.md"), raw_bytes)
        .expect("write remapped note");
    std::fs::write(target_root.join("notes").join("b.md"), sibling_bytes)
        .expect("write sibling note");

    let output = run_quaid(
        &db_path,
        &[
            "--json",
            "collection",
            "sync",
            "work",
            "--remap-root",
            target_root.to_str().expect("utf-8 target"),
        ],
    );

    assert!(
        output.status.success(),
        "offline remap should succeed: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert_eq!(parsed["resolved_pages"].as_u64(), Some(2));

    let conn = open_test_db(&db_path);
    let row: (String, String, i64, String) = conn
        .query_row(
            "SELECT c.state, c.root_path, c.needs_full_sync, fs.relative_path
             FROM collections c
             JOIN pages p ON p.collection_id = c.id AND p.slug = 'notes/a'
             JOIN file_state fs ON fs.page_id = p.id
             WHERE c.id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("load remapped collection");
    assert_eq!(row.0, "active");
    assert_eq!(row.1, target_root.to_str().expect("utf-8 target"));
    assert_eq!(row.2, 0);
    assert_eq!(row.3, "nested/renamed-a.md");
    assert_eq!(page_id(&conn, collection_id, "notes/a"), page_a);
    let link_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM links WHERE from_page_id = ?1 AND to_page_id = ?2",
            params![page_a, page_b],
            |row| row.get(0),
        )
        .expect("load preserved link count");
    assert_eq!(link_count, 1);
    assert_cli_lease_released(&conn, collection_id);
}

#[cfg(unix)]
#[test]
fn offline_remap_uses_hash_fallback_and_ignores_new_root_extras() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "offline-remap-hash-fallback-cli.db");
    let conn = open_test_db(&db_path);
    let source_root = dir.path().join("source");
    let target_root = dir.path().join("remapped");
    std::fs::create_dir_all(source_root.join("notes")).expect("create source root");
    let collection_id = insert_collection(&conn, "work", &source_root);
    let hash_fallback_bytes =
        b"---\nquaid_id: 11111111-1111-7111-8111-111111111111\nslug: notes/hash-fallback\ntitle: Hash Fallback\ntype: concept\n---\nthis body is intentionally long enough to cross the remap hash fallback threshold while still exercising the real CLI remap path end to end\n";
    let sibling_bytes =
        b"---\nmemory_id: 22222222-2222-7222-8222-222222222222\nquaid_id: 22222222-2222-7222-8222-222222222222\nslug: notes/b\ntitle: Sibling Note\ntype: concept\n---\nhello from sibling\n";
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/hash-fallback",
        "11111111-1111-7111-8111-111111111111",
        hash_fallback_bytes,
        "notes/hash-fallback.md",
    );
    insert_page_with_raw_import(
        &conn,
        collection_id,
        "notes/b",
        "22222222-2222-7222-8222-222222222222",
        sibling_bytes,
        "notes/b.md",
    );
    let fallback_page = page_id(&conn, collection_id, "notes/hash-fallback");
    let sibling_page = page_id(&conn, collection_id, "notes/b");
    insert_programmatic_link(&conn, fallback_page, sibling_page);
    std::fs::write(
        source_root.join("notes").join("hash-fallback.md"),
        hash_fallback_bytes,
    )
    .expect("seed source fallback note");
    std::fs::write(source_root.join("notes").join("b.md"), sibling_bytes)
        .expect("seed source sibling note");
    drop(conn);

    std::fs::create_dir_all(target_root.join("nested")).expect("create nested dir");
    std::fs::create_dir_all(target_root.join("notes")).expect("create notes dir");
    std::fs::create_dir_all(target_root.join("private")).expect("create ignored dir");
    std::fs::write(target_root.join(".quaidignore"), "private/**\n").expect("write ignore file");
    std::fs::write(
        target_root.join("nested").join("moved.md"),
        hash_fallback_bytes,
    )
    .expect("write moved fallback note");
    std::fs::write(target_root.join("notes").join("b.md"), sibling_bytes)
        .expect("write sibling note");
    std::fs::write(
        target_root.join("private").join("secret.md"),
        b"ignored secret",
    )
    .expect("write ignored secret");

    let output = run_quaid(
        &db_path,
        &[
            "--json",
            "collection",
            "sync",
            "work",
            "--remap-root",
            target_root.to_str().expect("utf-8 target"),
        ],
    );

    assert!(
        output.status.success(),
        "offline remap should honor hash fallback and .quaidignore extras: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert_eq!(parsed["resolved_pages"].as_u64(), Some(2));
    assert_eq!(parsed["missing_pages"].as_u64(), Some(0));
    assert_eq!(parsed["mismatched_pages"].as_u64(), Some(0));
    assert_eq!(parsed["extra_files"].as_u64(), Some(0));

    let conn = open_test_db(&db_path);
    let row: (String, String, i64, String) = conn
        .query_row(
            "SELECT c.state, c.root_path, c.needs_full_sync, fs.relative_path
             FROM collections c
             JOIN pages p ON p.collection_id = c.id AND p.slug = 'notes/hash-fallback'
             JOIN file_state fs ON fs.page_id = p.id
             WHERE c.id = ?1",
            [collection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("load remapped collection");
    assert_eq!(row.0, "active");
    assert_eq!(row.1, target_root.to_str().expect("utf-8 target"));
    assert_eq!(row.2, 0);
    assert_eq!(row.3, "nested/moved.md");
    assert_eq!(
        page_id(&conn, collection_id, "notes/hash-fallback"),
        fallback_page
    );
    let link_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM links WHERE from_page_id = ?1 AND to_page_id = ?2",
            params![fallback_page, sibling_page],
            |row| row.get(0),
        )
        .expect("load preserved link count");
    assert_eq!(link_count, 1);
    assert_cli_lease_released(&conn, collection_id);
}
