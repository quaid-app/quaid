use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use gbrain::core::db;
use gbrain::core::migrate::{export_dir, import_dir};
use rusqlite::Connection;
use sha2::{Digest, Sha256};

fn open_test_db(path: &Path) -> Connection {
    db::open(path.to_str().unwrap()).unwrap()
}

fn page_count(conn: &Connection) -> usize {
    conn.query_row("SELECT COUNT(*) FROM pages", [], |row| row.get::<_, i64>(0))
        .unwrap() as usize
}

fn exported_file_hashes(root: &Path) -> BTreeMap<String, String> {
    fn collect_hashes(root: &Path, dir: &Path, hashes: &mut BTreeMap<String, String>) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();

        for path in entries {
            if path.is_dir() {
                collect_hashes(root, &path, hashes);
            } else if path.extension().is_some_and(|ext| ext == "md") {
                let normalized = fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                let hash = format!("{:x}", Sha256::digest(normalized.as_bytes()));
                hashes.insert(relative, hash);
            }
        }
    }

    let mut hashes = BTreeMap::new();
    collect_hashes(root, root, &mut hashes);
    hashes
}

#[test]
fn import_export_reimport_preserves_page_count_and_rendered_content_hashes() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");

    let source_db_dir = tempfile::TempDir::new().unwrap();
    let source_conn = open_test_db(&source_db_dir.path().join("source.db"));
    let initial_stats = import_dir(&source_conn, &fixtures_dir, false).unwrap();
    let original_page_count = page_count(&source_conn);

    assert_eq!(initial_stats.imported, original_page_count);

    let first_export_root = tempfile::TempDir::new().unwrap();
    let exported_count = export_dir(&source_conn, first_export_root.path()).unwrap();
    assert_eq!(exported_count, original_page_count);
    let first_export_hashes = exported_file_hashes(first_export_root.path());

    let roundtrip_db_dir = tempfile::TempDir::new().unwrap();
    let roundtrip_conn = open_test_db(&roundtrip_db_dir.path().join("roundtrip.db"));
    import_dir(&roundtrip_conn, first_export_root.path(), false).unwrap();
    assert_eq!(page_count(&roundtrip_conn), original_page_count);

    let second_export_root = tempfile::TempDir::new().unwrap();
    let second_exported_count = export_dir(&roundtrip_conn, second_export_root.path()).unwrap();
    assert_eq!(second_exported_count, original_page_count);

    let second_export_hashes = exported_file_hashes(second_export_root.path());
    assert_eq!(second_export_hashes, first_export_hashes);
}
