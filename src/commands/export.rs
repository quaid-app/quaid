use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

use crate::core::migrate;

pub fn run(db: &Connection, path: &str, _raw: bool, _import_id: Option<String>) -> Result<()> {
    let output = Path::new(path);
    let count = migrate::export_dir(db, output)?;
    println!("Exported {count} page(s) to {path}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{commands::put, core::db};
    use std::fs;

    fn open_test_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        // Provision a real vault root so the Unix persist path can open
        // the collection root directory.  The default collection is seeded
        // with root_path='' (state='detached') which is only valid for
        // in-memory usage; on-disk writes require a real directory.
        let vault_root = dir.path().join("vault");
        fs::create_dir_all(&vault_root).unwrap();
        conn.execute(
            "UPDATE collections
             SET root_path = ?1,
                 writable = 1,
                 is_write_target = 1,
                 state = 'active',
                 needs_full_sync = 0
             WHERE id = 1",
            [vault_root.display().to_string()],
        )
        .unwrap();
        (dir, conn)
    }

    #[test]
    fn run_exports_page_to_nested_markdown_file() {
        let (dir, conn) = open_test_db();
        put::put_from_string(
            &conn,
            "notes/example",
            "---\ntitle: Example Export\n---\nExport body\n",
            None,
        )
        .unwrap();

        let export_dir = dir.path().join("exported");
        run(
            &conn,
            export_dir.to_str().expect("export path"),
            false,
            None,
        )
        .unwrap();

        let exported = export_dir.join("notes").join("example.md");
        let contents = fs::read_to_string(exported).unwrap();
        assert!(contents.contains("title: Example Export"));
        assert!(contents.contains("Export body"));
    }
}
