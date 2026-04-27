#[cfg(unix)]
mod watcher_core {
    use quaid::core::{db, vault_sync};
    use rusqlite::{params, Connection};
    use sha2::Digest;
    use std::path::{Path, PathBuf};

    #[derive(Debug, PartialEq, Eq)]
    struct PageSnapshot {
        page_count: i64,
        file_state_count: i64,
        raw_import_count: i64,
        embedding_job_count: i64,
        state: String,
        pending_root_path: Option<String>,
        restore_command_id: Option<String>,
        slug: String,
        compiled_truth: String,
        version: i64,
        relative_path: String,
        sha256: String,
        size_bytes: i64,
    }

    fn open_test_db(path: &Path) -> Connection {
        db::open(path.to_str().expect("utf-8 db path")).expect("open test db")
    }

    fn insert_collection(conn: &Connection, name: &str, root_path: &Path) -> i64 {
        conn.execute(
            "INSERT INTO collections (name, root_path, state, writable, is_write_target)
             VALUES (?1, ?2, 'active', 1, 0)",
            params![name, root_path.display().to_string()],
        )
        .expect("insert collection");
        conn.last_insert_rowid()
    }

    fn insert_page_with_raw_import(
        conn: &Connection,
        collection_id: i64,
        slug: &str,
        truth: &str,
        raw_bytes: &[u8],
        relative_path: &str,
    ) {
        let page_uuid = uuid::Uuid::now_v7().to_string();
        conn.execute(
            "INSERT INTO pages
                 (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
             VALUES (?1, ?2, ?3, 'concept', ?2, '', ?4, '', '{}', 'notes', '', 1)",
            params![collection_id, slug, page_uuid, truth],
        )
        .expect("insert page");
        let page_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO raw_imports (page_id, import_id, is_active, raw_bytes, file_path)
             VALUES (?1, ?2, 1, ?3, ?4)",
            params![
                page_id,
                uuid::Uuid::now_v7().to_string(),
                raw_bytes,
                relative_path
            ],
        )
        .expect("insert raw import");
        let sha256 = format!("{:x}", sha2::Sha256::digest(raw_bytes));
        conn.execute(
            "INSERT INTO file_state (collection_id, relative_path, page_id, mtime_ns, ctime_ns, size_bytes, inode, sha256)
             VALUES (?1, ?2, ?3, 1, 1, ?4, 1, ?5)",
            params![
                collection_id,
                relative_path,
                page_id,
                raw_bytes.len() as i64,
                sha256
            ],
        )
        .expect("insert file state");
    }

    fn page_snapshot(conn: &Connection, collection_id: i64) -> PageSnapshot {
        conn.query_row(
            "SELECT
                 (SELECT COUNT(*) FROM pages),
                 (SELECT COUNT(*) FROM file_state),
                 (SELECT COUNT(*) FROM raw_imports),
                 (SELECT COUNT(*) FROM embedding_jobs),
                 c.state,
                 c.pending_root_path,
                 c.restore_command_id,
                 p.slug,
                 p.compiled_truth,
                 p.version,
                 f.relative_path,
                 f.sha256,
                 f.size_bytes
             FROM collections c
             JOIN pages p ON p.collection_id = c.id
             JOIN file_state f ON f.page_id = p.id
             WHERE c.id = ?1",
            [collection_id],
            |row| {
                Ok(PageSnapshot {
                    page_count: row.get(0)?,
                    file_state_count: row.get(1)?,
                    raw_import_count: row.get(2)?,
                    embedding_job_count: row.get(3)?,
                    state: row.get(4)?,
                    pending_root_path: row.get(5)?,
                    restore_command_id: row.get(6)?,
                    slug: row.get(7)?,
                    compiled_truth: row.get(8)?,
                    version: row.get(9)?,
                    relative_path: row.get(10)?,
                    sha256: row.get(11)?,
                    size_bytes: row.get(12)?,
                })
            },
        )
        .expect("load page snapshot")
    }

    fn test_db_path(dir: &tempfile::TempDir, name: &str) -> PathBuf {
        dir.path().join(name)
    }

    #[test]
    fn start_serve_runtime_defers_fresh_restore_without_mutating_page_rows() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let db_path = test_db_path(&dir, "watcher-core-deferred.db");
        let conn = open_test_db(&db_path);
        let root = dir.path().join("vault");
        let pending_root = dir.path().join("restored");
        std::fs::create_dir_all(root.join("notes")).expect("create root notes");
        std::fs::create_dir_all(&pending_root).expect("create pending root");

        let collection_id = insert_collection(&conn, "work", &root);
        let raw_bytes = b"# watcher proof\n";
        insert_page_with_raw_import(
            &conn,
            collection_id,
            "watcher-proof",
            "compiled watcher proof",
            raw_bytes,
            "notes/watcher-proof.md",
        );
        conn.execute(
            "UPDATE collections
             SET state = 'restoring',
                 pending_root_path = ?2,
                 pending_restore_manifest = '{\"entries\":[]}',
                 restore_command_id = 'restore-1',
                 pending_command_heartbeat_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?1",
            params![collection_id, pending_root.display().to_string()],
        )
        .expect("seed fresh restoring collection");
        let before = page_snapshot(&conn, collection_id);
        drop(conn);

        let runtime =
            vault_sync::start_serve_runtime(db_path.to_str().expect("utf-8 db path").to_owned())
                .expect("start serve runtime");

        let conn = open_test_db(&db_path);
        let after = page_snapshot(&conn, collection_id);

        assert_eq!(after, before);
        assert_eq!(after.state, "restoring");
        assert_eq!(
            after.pending_root_path,
            Some(pending_root.display().to_string())
        );
        assert_eq!(after.restore_command_id.as_deref(), Some("restore-1"));

        drop(conn);
        drop(runtime);
    }
}
