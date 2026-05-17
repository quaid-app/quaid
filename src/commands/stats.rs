#![expect(
    clippy::print_stdout,
    reason = "CLI command prints user-facing output to stdout by design"
)]

use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

/// Memory statistics summary.
#[derive(Debug, Serialize)]
struct MemoryStats {
    total_pages: i64,
    pages_by_type: Vec<TypeCount>,
    total_links: i64,
    total_embeddings: i64,
    fts_rows: i64,
    collection_totals: CollectionTotals,
    collections: Vec<CollectionStats>,
    db_size_bytes: u64,
}

#[derive(Debug, Serialize)]
struct TypeCount {
    page_type: String,
    count: i64,
}

#[derive(Debug, Serialize)]
struct CollectionTotals {
    total_pages: i64,
    quarantined_pages: i64,
    embedding_jobs_pending: i64,
    embedding_jobs_failed: i64,
}

#[derive(Debug, Serialize)]
struct CollectionStats {
    name: String,
    page_count: i64,
    queue_depth: i64,
    last_sync_at: Option<String>,
    state: String,
    writable: bool,
}

/// Gather and print memory statistics.
pub fn run(db: &Connection, json: bool) -> Result<()> {
    let stats = gather_stats(db)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!("Pages:      {}", stats.total_pages);
        if !stats.pages_by_type.is_empty() {
            for tc in &stats.pages_by_type {
                println!("  {}: {}", tc.page_type, tc.count);
            }
        }
        println!("Links:      {}", stats.total_links);
        println!("Embeddings: {}", stats.total_embeddings);
        println!("FTS rows:   {}", stats.fts_rows);
        println!(
            "Collections: total_pages={} quarantined_pages={} embedding_jobs_pending={} embedding_jobs_failed={}",
            stats.collection_totals.total_pages,
            stats.collection_totals.quarantined_pages,
            stats.collection_totals.embedding_jobs_pending,
            stats.collection_totals.embedding_jobs_failed
        );
        if !stats.collections.is_empty() {
            println!("name | page_count | queue_depth | last_sync_at | state | writable");
            for collection in &stats.collections {
                println!(
                    "{} | {} | {} | {} | {} | {}",
                    collection.name,
                    collection.page_count,
                    collection.queue_depth,
                    collection.last_sync_at.as_deref().unwrap_or("-"),
                    collection.state,
                    collection.writable
                );
            }
        }
        println!(
            "DB size:    {:.2} MB",
            stats.db_size_bytes as f64 / 1_048_576.0
        );
    }

    Ok(())
}

fn gather_stats(db: &Connection) -> Result<MemoryStats> {
    let total_pages: i64 = db.query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))?;

    let mut stmt = db.prepare("SELECT type, COUNT(*) FROM pages GROUP BY type ORDER BY type")?;
    let pages_by_type: Vec<TypeCount> = stmt
        .query_map([], |row| {
            Ok(TypeCount {
                page_type: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .filter_map(Result::ok)
        .collect();

    let total_links: i64 = db.query_row("SELECT COUNT(*) FROM links", [], |row| row.get(0))?;

    let total_embeddings: i64 =
        db.query_row("SELECT COUNT(*) FROM page_embeddings", [], |row| row.get(0))?;

    let fts_rows: i64 = db.query_row("SELECT COUNT(*) FROM page_fts", [], |row| row.get(0))?;

    let collection_totals = db.query_row(
        "SELECT
             COALESCE((SELECT COUNT(*) FROM pages), 0),
             COALESCE((SELECT COUNT(*) FROM pages WHERE quarantined_at IS NOT NULL), 0),
             COALESCE((SELECT COUNT(*) FROM embedding_jobs WHERE job_state IN ('pending', 'running')), 0),
             COALESCE((SELECT COUNT(*) FROM embedding_jobs WHERE job_state = 'failed'), 0)",
        [],
        |row| {
            Ok(CollectionTotals {
                total_pages: row.get(0)?,
                quarantined_pages: row.get(1)?,
                embedding_jobs_pending: row.get(2)?,
                embedding_jobs_failed: row.get(3)?,
            })
        },
    )?;

    let mut stmt = db.prepare(
        "SELECT c.name,
                COALESCE((
                    SELECT COUNT(*)
                    FROM pages p
                    WHERE p.collection_id = c.id AND p.quarantined_at IS NULL
                ), 0),
                COALESCE((
                    SELECT COUNT(*)
                    FROM embedding_jobs ej
                    JOIN pages p ON p.id = ej.page_id
                    WHERE p.collection_id = c.id
                      AND ej.job_state IN ('pending', 'running')
                ), 0),
                c.last_sync_at,
                c.state,
                c.writable
         FROM collections c
         WHERE c.root_path <> ''
         ORDER BY c.name",
    )?;
    let collections: Vec<CollectionStats> = stmt
        .query_map([], |row| {
            Ok(CollectionStats {
                name: row.get(0)?,
                page_count: row.get(1)?,
                queue_depth: row.get(2)?,
                last_sync_at: row.get(3)?,
                state: row.get(4)?,
                writable: row.get::<_, i64>(5)? != 0,
            })
        })?
        .filter_map(Result::ok)
        .collect();

    // DB file size via the database path from PRAGMA database_list
    let db_path: String = db.query_row(
        "SELECT file FROM pragma_database_list WHERE name = 'main'",
        [],
        |row| row.get(0),
    )?;
    let db_size_bytes = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);

    Ok(MemoryStats {
        total_pages,
        pages_by_type,
        total_links,
        total_embeddings,
        fts_rows,
        collection_totals,
        collections,
        db_size_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;

    fn open_test_db() -> Connection {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        std::mem::forget(dir);
        conn
    }

    fn insert_page(conn: &Connection, slug: &str, page_type: &str) {
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version) \
             VALUES (?1, ?2, ?3, '', '', '', '{}', '', '', 1)",
            rusqlite::params![slug, page_type, slug],
        )
        .unwrap();
    }

    #[test]
    fn stats_on_empty_db_returns_zeros() {
        let conn = open_test_db();
        let stats = gather_stats(&conn).unwrap();

        assert_eq!(stats.total_pages, 0);
        assert!(stats.pages_by_type.is_empty());
        assert_eq!(stats.total_links, 0);
        assert_eq!(stats.total_embeddings, 0);
        assert_eq!(stats.fts_rows, 0);
        assert_eq!(stats.collection_totals.total_pages, 0);
        assert_eq!(stats.collection_totals.quarantined_pages, 0);
        assert_eq!(stats.collection_totals.embedding_jobs_pending, 0);
        assert_eq!(stats.collection_totals.embedding_jobs_failed, 0);
        assert_eq!(stats.collections.len(), 1);
        assert_eq!(stats.collections[0].name, "default");
        assert_eq!(stats.collections[0].page_count, 0);
    }

    #[test]
    fn stats_counts_pages_and_types() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person");
        insert_page(&conn, "people/bob", "person");
        insert_page(&conn, "companies/acme", "company");

        let stats = gather_stats(&conn).unwrap();

        assert_eq!(stats.total_pages, 3);
        assert_eq!(stats.pages_by_type.len(), 2);

        let company = stats
            .pages_by_type
            .iter()
            .find(|t| t.page_type == "company")
            .unwrap();
        assert_eq!(company.count, 1);

        let person = stats
            .pages_by_type
            .iter()
            .find(|t| t.page_type == "person")
            .unwrap();
        assert_eq!(person.count, 2);
    }

    #[test]
    fn stats_counts_fts_rows_from_triggers() {
        let conn = open_test_db();
        insert_page(&conn, "test/one", "concept");
        insert_page(&conn, "test/two", "concept");

        let stats = gather_stats(&conn).unwrap();
        // FTS5 triggers fire on insert, so rows should match pages
        assert_eq!(stats.fts_rows, 2);
    }

    #[test]
    fn stats_reports_nonzero_db_file_size() {
        let conn = open_test_db();
        insert_page(&conn, "test/size", "concept");

        let stats = gather_stats(&conn).unwrap();
        assert!(stats.db_size_bytes > 0, "DB file size should be non-zero");
    }

    #[test]
    fn stats_run_succeeds_for_text_and_json_output() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person");
        insert_page(&conn, "people/bob", "person");
        conn.execute(
            "UPDATE collections
             SET root_path = 'C:\\vault',
                 state = 'active',
                 writable = 1,
                 last_sync_at = '2026-05-02T00:00:00Z'
             WHERE name = 'default'",
            [],
        )
        .unwrap();
        let alice_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE slug = 'people/alice'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let bob_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE slug = 'people/bob'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind) VALUES (?1, ?2, 'knows', 'programmatic')",
            rusqlite::params![alice_id, bob_id],
        )
        .unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        conn.execute(
            "INSERT INTO page_embeddings (page_id, model, vec_rowid, chunk_type, chunk_index, chunk_text, content_hash, token_count) \
             VALUES (?1, 'BAAI/bge-small-en-v1.5', 1, 'truth_section', 0, 'text', 'abc', 1)",
            [alice_id],
        )
        .unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();

        run(&conn, false).unwrap();
        run(&conn, true).unwrap();
    }

    #[test]
    fn stats_includes_collection_rows_and_aggregate_totals() {
        let conn = open_test_db();
        conn.execute(
            "UPDATE collections
             SET root_path = 'C:\\vault\\work',
                 state = 'active',
                 writable = 1,
                 last_sync_at = '2026-05-02T00:00:00Z'
             WHERE name = 'default'",
            [],
        )
        .unwrap();
        let collection_id: i64 = conn
            .query_row(
                "SELECT id FROM collections WHERE name = 'default'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        insert_page(&conn, "work/note", "concept");
        let page_id: i64 = conn
            .query_row("SELECT id FROM pages WHERE slug = 'work/note'", [], |row| {
                row.get(0)
            })
            .unwrap();
        conn.execute(
            "UPDATE pages
             SET collection_id = ?1,
                 quarantined_at = '2026-05-02T00:00:01Z'
             WHERE id = ?2",
            rusqlite::params![collection_id, page_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO embedding_jobs (page_id, priority, job_state)
             VALUES (?1, 1, 'failed')",
            [page_id],
        )
        .unwrap();

        let stats = gather_stats(&conn).unwrap();

        assert_eq!(stats.collection_totals.total_pages, 1);
        assert_eq!(stats.collection_totals.quarantined_pages, 1);
        assert_eq!(stats.collection_totals.embedding_jobs_pending, 0);
        assert_eq!(stats.collection_totals.embedding_jobs_failed, 1);
        assert_eq!(stats.collections.len(), 1);
        assert_eq!(stats.collections[0].name, "default");
        assert_eq!(stats.collections[0].page_count, 0);
        assert_eq!(stats.collections[0].queue_depth, 0);
        assert_eq!(stats.collections[0].state, "active");
        assert!(stats.collections[0].writable);
    }

    #[test]
    fn stats_in_memory_database_reports_zero_file_size() {
        let conn = db::open(":memory:").unwrap();

        let stats = gather_stats(&conn).unwrap();

        assert_eq!(stats.db_size_bytes, 0);
    }
}
