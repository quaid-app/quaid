#[cfg(unix)]
mod watcher_core {
    use quaid::core::{db, fts, vault_sync};
    use rusqlite::{params, Connection, OpenFlags};
    use sha2::Digest;
    use std::path::{Path, PathBuf};
    use std::thread;
    use std::time::{Duration, Instant};

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
        let started = Instant::now();
        loop {
            match db::open(path.to_str().expect("utf-8 db path")) {
                Ok(conn) => return conn,
                Err(quaid::core::types::DbError::Sqlite(rusqlite::Error::SqliteFailure(
                    err,
                    _,
                ))) if err.code == rusqlite::ErrorCode::DatabaseBusy
                    && started.elapsed() < Duration::from_secs(5) =>
                {
                    thread::sleep(Duration::from_millis(25));
                }
                Err(err) => panic!("open test db: {err:?}"),
            }
        }
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
            "INSERT INTO raw_imports (page_id, import_id, is_active, content_hash, raw_bytes, file_path)
             VALUES (?1, ?2, 1, ?3, ?4, ?5)",
            params![
                page_id,
                uuid::Uuid::now_v7().to_string(),
                format!("{:x}", sha2::Sha256::digest(raw_bytes)),
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

    fn page_id(conn: &Connection, collection_id: i64, slug: &str) -> i64 {
        conn.query_row(
            "SELECT id FROM pages WHERE collection_id = ?1 AND slug = ?2",
            params![collection_id, slug],
            |row| row.get(0),
        )
        .expect("load page id")
    }

    fn wait_for_db_value<T>(
        db_path: &Path,
        timeout: Duration,
        mut probe: impl FnMut(&Connection) -> Option<T>,
    ) -> Option<T> {
        let started = Instant::now();
        while started.elapsed() <= timeout {
            let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .expect("open read-only poll db");
            if let Some(value) = probe(&conn) {
                return Some(value);
            }
            drop(conn);
            thread::sleep(Duration::from_millis(25));
        }
        None
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

    #[test]
    fn watcher_observes_warm_up_edit_after_runtime_start() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let db_path = test_db_path(&dir, "watcher-latency.db");
        let conn = open_test_db(&db_path);
        let root = dir.path().join("vault");
        std::fs::create_dir_all(root.join("notes")).expect("create root notes");
        let collection_id = insert_collection(&conn, "work", &root);
        let note_path = root.join("notes").join("latency.md");
        std::fs::write(
            &note_path,
            b"---\ntitle: Latency\ntype: note\n---\noriginal watcher body\n",
        )
        .expect("seed note");
        vault_sync::sync_collection(&conn, "work").expect("initial sync");
        drop(conn);

        let runtime =
            vault_sync::start_serve_runtime(db_path.to_str().expect("utf-8 db path").to_owned())
                .expect("start serve runtime");

        std::fs::write(
            &note_path,
            b"---\ntitle: Latency\ntype: note\n---\nwarm watcher body\n",
        )
        .expect("write warm-up edit");
        let warmed = wait_for_db_value(&db_path, Duration::from_secs(10), |verify| {
            verify
                .query_row(
                    "SELECT compiled_truth
                     FROM pages
                     WHERE collection_id = ?1 AND slug = 'notes/latency'",
                    [collection_id],
                    |row| row.get::<_, String>(0),
                )
                .ok()
                .and_then(|compiled_truth| {
                    compiled_truth.contains("warm watcher body").then_some(())
                })
        });
        assert!(
            warmed.is_some(),
            "watcher never observed the warm-up edit before the latency assertion"
        );

        drop(runtime);
    }

    #[test]
    fn semantic_search_is_fts_fresh_while_embedding_lane_catches_up() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let db_path = test_db_path(&dir, "watcher-semantic-consistency.db");
        let conn = open_test_db(&db_path);
        let root = dir.path().join("vault");
        std::fs::create_dir_all(root.join("notes")).expect("create root notes");
        let collection_id = insert_collection(&conn, "work", &root);
        let note_path = root.join("notes").join("search-lag.md");
        std::fs::write(
            &note_path,
            b"---\ntitle: Search Lag\ntype: note\n---\ngarden soil compost tomatoes\n",
        )
        .expect("seed note");
        vault_sync::sync_collection(&conn, "work").expect("initial sync");
        let page_id = page_id(&conn, collection_id, "notes/search-lag");
        vault_sync::drain_embedding_queue(&conn).expect("seed initial embeddings");
        let initial_chunk: String = conn
            .query_row(
                "SELECT chunk_text
                 FROM page_embeddings
                 WHERE page_id = ?1
                 ORDER BY chunk_index
                 LIMIT 1",
                [page_id],
                |row| row.get(0),
            )
            .expect("load initial embedding chunk");
        assert!(initial_chunk.contains("garden soil compost tomatoes"));
        drop(conn);

        let runtime =
            vault_sync::start_serve_runtime(db_path.to_str().expect("utf-8 db path").to_owned())
                .expect("start serve runtime");
        thread::sleep(Duration::from_millis(300));

        let repeated = "orbital rendezvous burn window ".repeat(512);
        let updated_markdown = format!(
            "---\ntitle: Search Lag\ntype: note\n---\norbital rendezvous burn window\n{repeated}\n"
        );
        std::fs::write(&note_path, updated_markdown).expect("write updated note");

        let phase_one = wait_for_db_value(&db_path, Duration::from_secs(10), |verify| {
            let fts_results = fts::search_fts_canonical_tiered(
                "orbital rendezvous burn window",
                None,
                Some(collection_id),
                verify,
                5,
            )
            .ok()?;
            let embedding_jobs: i64 = verify
                .query_row(
                    "SELECT COUNT(*)
                     FROM embedding_jobs
                     WHERE page_id = ?1 AND job_state IN ('pending', 'running')",
                    [page_id],
                    |row| row.get(0),
                )
                .ok()?;
            let chunk_text: String = verify
                .query_row(
                    "SELECT chunk_text
                     FROM page_embeddings
                     WHERE page_id = ?1
                     ORDER BY chunk_index
                     LIMIT 1",
                    [page_id],
                    |row| row.get(0),
                )
                .ok()?;
            (fts_results
                .iter()
                .any(|result| result.slug == "work::notes/search-lag")
                && embedding_jobs > 0
                && !chunk_text.contains("orbital rendezvous burn window"))
            .then_some(())
        });

        assert!(
            phase_one.is_some(),
            "FTS never became fresh ahead of the embedding lane"
        );

        drop(runtime);
        let conn = open_test_db(&db_path);
        vault_sync::drain_embedding_queue(&conn).expect("drain embedding queue");
        drop(conn);

        let phase_two = wait_for_db_value(&db_path, Duration::from_secs(20), |verify| {
            let embedding_jobs: i64 = verify
                .query_row(
                    "SELECT COUNT(*)
                     FROM embedding_jobs
                     WHERE page_id = ?1 AND job_state IN ('pending', 'running')",
                    [page_id],
                    |row| row.get(0),
                )
                .ok()?;
            let chunk_text: String = verify
                .query_row(
                    "SELECT chunk_text
                     FROM page_embeddings
                     WHERE page_id = ?1
                     ORDER BY chunk_index
                     LIMIT 1",
                    [page_id],
                    |row| row.get(0),
                )
                .ok()?;
            (embedding_jobs == 0 && chunk_text.contains("orbital rendezvous burn window"))
                .then_some(chunk_text)
        });

        let updated_chunk = phase_two.expect("embedding lane never caught up");
        assert!(updated_chunk.contains("orbital rendezvous burn window"));
    }
}
