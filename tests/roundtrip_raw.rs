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

/// `quaid export --raw` restores the byte-exact active raw payload even when
/// the source bytes are NOT in canonical form (so the rendered export would
/// differ from them).
#[test]
fn export_raw_reproduces_non_canonical_source_bytes_exactly() {
    // Trailing whitespace, CRLF, and unsorted frontmatter keys — content the
    // normalized renderer would rewrite.
    let messy = "---\ntype: person\ntitle: Messy Person\nslug: notes/messy-person\n---\r\nBody with trailing spaces   \r\n\r\n";

    let fixture_root = tempfile::TempDir::new().unwrap();
    let fixture_path = fixture_root.path().join("messy.md");
    fs::write(&fixture_path, messy).unwrap();

    let db_root = tempfile::TempDir::new().unwrap();
    let db_path = db_root.path().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    ingest::run(&conn, fixture_path.to_str().unwrap(), false).unwrap();

    let export_root = tempfile::TempDir::new().unwrap();
    let count = quaid::core::migrate::export_raw_dir(&conn, export_root.path(), None).unwrap();
    assert_eq!(count, 1);

    let exported = fs::read(export_root.path().join("notes/messy-person.md")).unwrap();
    assert_eq!(exported, messy.as_bytes());
}

/// `--import-id` restores exactly one historical payload by its
/// `raw_imports.import_id`, and unknown ids error instead of silently
/// writing nothing.
#[test]
fn export_raw_with_import_id_restores_single_historical_payload() {
    let first = "---\ntitle: Note\ntype: concept\nslug: notes/history\n---\nFirst revision.\n";
    let second = "---\ntitle: Note\ntype: concept\nslug: notes/history\n---\nSecond revision.\n";

    let fixture_root = tempfile::TempDir::new().unwrap();
    let db_root = tempfile::TempDir::new().unwrap();
    let db_path = db_root.path().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();

    let first_path = fixture_root.path().join("a.md");
    fs::write(&first_path, first).unwrap();
    ingest::run(&conn, first_path.to_str().unwrap(), false).unwrap();
    let second_path = fixture_root.path().join("b.md");
    fs::write(&second_path, second).unwrap();
    ingest::run(&conn, second_path.to_str().unwrap(), true).unwrap();

    // The first revision rotated to inactive; grab its import id.
    let historic_import_id: String = conn
        .query_row(
            "SELECT import_id FROM raw_imports WHERE is_active = 0 LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let export_root = tempfile::TempDir::new().unwrap();
    let count =
        quaid::core::migrate::export_raw_dir(&conn, export_root.path(), Some(&historic_import_id))
            .unwrap();
    assert_eq!(count, 1);
    let exported = fs::read(export_root.path().join("notes/history.md")).unwrap();
    assert_eq!(exported, first.as_bytes());

    let missing = quaid::core::migrate::export_raw_dir(
        &conn,
        export_root.path(),
        Some("00000000-0000-0000-0000-000000000000"),
    );
    assert!(missing.is_err(), "unknown import id must error");
    assert!(missing
        .unwrap_err()
        .to_string()
        .contains("raw import not found"));
}

/// The CLI command surface: `--raw` flows through `commands::export::run`
/// (formerly bound as `_raw`/`_import_id` no-ops), and `--import-id`
/// without `--raw` is rejected.
#[test]
fn export_command_honours_raw_flag_and_guards_import_id() {
    let canonical = "---\ntitle: Cli Raw\ntype: concept\nslug: notes/cli-raw\n---\nBody.   \n";
    let fixture_root = tempfile::TempDir::new().unwrap();
    let fixture_path = fixture_root.path().join("cli-raw.md");
    fs::write(&fixture_path, canonical).unwrap();

    let db_root = tempfile::TempDir::new().unwrap();
    let db_path = db_root.path().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    ingest::run(&conn, fixture_path.to_str().unwrap(), false).unwrap();

    let export_root = tempfile::TempDir::new().unwrap();
    quaid::commands::export::run(
        &conn,
        export_root.path().to_str().unwrap(),
        true,
        None,
        false,
    )
    .unwrap();
    let exported = fs::read(export_root.path().join("notes/cli-raw.md")).unwrap();
    assert_eq!(exported, canonical.as_bytes());

    let guard = quaid::commands::export::run(
        &conn,
        export_root.path().to_str().unwrap(),
        false,
        Some("some-id".to_owned()),
        false,
    );
    assert!(guard.is_err(), "--import-id without --raw must error");
}
