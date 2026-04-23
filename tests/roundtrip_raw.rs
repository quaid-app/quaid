use std::fs;

use gbrain::core::db;
use gbrain::core::migrate::{export_dir, import_dir};

#[test]
fn export_reproduces_canonical_markdown_fixture_byte_for_byte() {
    let canonical = concat!(
        "---\n",
        "gbrain_id: 01969f11-9448-7d79-8d3f-c68f54761234\n",
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
    let db_path = db_root.path().join("brain.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();

    let import_stats = import_dir(&conn, fixture_root.path(), false).unwrap();
    assert_eq!(import_stats.imported, 1);

    let export_root = tempfile::TempDir::new().unwrap();
    let exported_count = export_dir(&conn, export_root.path()).unwrap();
    assert_eq!(exported_count, 1);

    let exported = fs::read(export_root.path().join("notes/canonical-person.md")).unwrap();
    assert_eq!(exported, canonical.as_bytes());
}
