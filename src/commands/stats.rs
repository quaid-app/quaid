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
    db_size_bytes: u64,
}

#[derive(Debug, Serialize)]
struct TypeCount {
    page_type: String,
    count: i64,
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
    fn stats_in_memory_database_reports_zero_file_size() {
        let conn = db::open(":memory:").unwrap();

        let stats = gather_stats(&conn).unwrap();

        assert_eq!(stats.db_size_bytes, 0);
    }
}
