#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

use std::fs;

use quaid::commands::ingest;
use quaid::core::db;
use quaid::core::migrate::export_dir;

#[test]
fn export_reproduces_canonical_markdown_fixture_byte_for_byte() {
    let canonical = concat!(
        "---\n",
        "quaid_id: 01969f11-9448-7d79-8d3f-c68f54761234\n",
        "slug: notes/canonical-person\n",
        "title: Canonical Person\n",
        "type: person\n",
        "wing: notes\n",
        "---\n",
        "# Canonical Person\n\n",
        "Canonical truth paragraph.\n",
        "---\n",
        "- **2026-04** | note — Added canonical fixture."
    );

    let fixture_root = tempfile::TempDir::new().unwrap();
    let fixture_path = fixture_root.path().join("notes/canonical-person.md");
    fs::create_dir_all(fixture_path.parent().unwrap()).unwrap();
    fs::write(&fixture_path, canonical).unwrap();

    let db_root = tempfile::TempDir::new().unwrap();
    let db_path = db_root.path().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();

    ingest::run(&conn, fixture_path.to_str().unwrap(), false).unwrap();
    let page_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(page_count, 1);

    let export_root = tempfile::TempDir::new().unwrap();
    let exported_count = export_dir(&conn, export_root.path()).unwrap();
    assert_eq!(exported_count, 1);

    let exported = fs::read(export_root.path().join("notes/canonical-person.md")).unwrap();
    assert_eq!(exported, canonical.as_bytes());
}
