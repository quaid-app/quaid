//! Knowledge-gap log: records queries the brain couldn't answer (keyed by
//! SHA-256 for idempotent inserts), lists unresolved entries, and links them
//! to pages that later answered them. Provides the data behind `quaid gaps`
//! (including `quaid gaps resolve`) and the matching MCP tools
//! (`memory_gap`, `memory_gaps`, `memory_gap_resolve`).
//!
//! See also: `types::KnowledgeGap` for the row shape, and `search` for the
//! query path that detects low-confidence retrievals.

use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::types::{KnowledgeGap, SearchResult};

/// Failure mode raised by gap logging, listing, or resolution.
#[derive(Debug, Error)]
pub enum GapsError {
    /// Underlying SQLite failure.
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// No unresolved gap exists with the requested id.
    #[error("gap not found: id {id}")]
    NotFound {
        /// The requested gap id.
        id: i64,
    },
}

/// Decide whether a retrieval outcome should be auto-logged as a knowledge
/// gap.
///
/// A top result scoring `>= 0.99` is a perfect hit (the exact-slug
/// short-circuit returns `1.0`, and a top-normalized strong hybrid hit
/// approaches `1.0` under both merge strategies) and is never a gap.
/// Otherwise a gap is logged when the result set is empty or every result
/// scores below `0.3`. Both hybrid merge strategies emit scores on a
/// `[0, 1]` scale, so the single threshold applies to either.
pub fn should_log_gap(results: &[SearchResult]) -> bool {
    if results.first().is_some_and(|result| result.score >= 0.99) {
        return false;
    }
    results.is_empty() || results.iter().all(|result| result.score < 0.3)
}

/// Build the machine-generated, query-free diagnostics string recorded as
/// the `context` of an auto-logged gap (e.g. `auto: hybrid_search results=3
/// top=0.02`). Never includes query text, preserving the privacy-by-default
/// posture of the gap log.
pub fn auto_gap_context(source: &str, results: &[SearchResult]) -> String {
    match results.first() {
        Some(top) => format!(
            "auto: {source} results={} top={:.2}",
            results.len(),
            top.score
        ),
        None => format!("auto: {source} results=0"),
    }
}

/// Log a knowledge gap using the SHA-256 of the query for idempotency.
///
/// Uses `INSERT OR IGNORE` against the UNIQUE index on `query_hash`
/// so duplicate queries produce exactly one gap row.
pub fn log_gap(
    page_id: Option<i64>,
    query: &str,
    context: &str,
    confidence_score: Option<f64>,
    conn: &Connection,
) -> Result<(), GapsError> {
    let hash = sha256_hex(query);
    conn.execute(
        "INSERT OR IGNORE INTO knowledge_gaps (page_id, query_hash, context, confidence_score, sensitivity) \
         VALUES (?1, ?2, ?3, ?4, 'internal')",
        params![page_id, hash, context, confidence_score],
    )?;
    Ok(())
}

/// Convenience wrapper around [`log_gap`] for callers that always have a
/// concrete `page_id` (as opposed to a free-floating query).
pub fn log_gap_for_page(
    page_id: i64,
    query: &str,
    context: &str,
    confidence_score: Option<f64>,
    conn: &Connection,
) -> Result<(), GapsError> {
    log_gap(Some(page_id), query, context, confidence_score, conn)
}

/// List knowledge gaps, optionally including resolved ones.
pub fn list_gaps(
    resolved: bool,
    limit: usize,
    conn: &Connection,
) -> Result<Vec<KnowledgeGap>, GapsError> {
    let sql = if resolved {
        "SELECT id, page_id, query_hash, context, confidence_score, sensitivity, resolved_at, resolved_by_slug, detected_at \
         FROM knowledge_gaps ORDER BY detected_at DESC LIMIT ?1"
    } else {
        "SELECT id, page_id, query_hash, context, confidence_score, sensitivity, resolved_at, resolved_by_slug, detected_at \
         FROM knowledge_gaps WHERE resolved_at IS NULL ORDER BY detected_at DESC LIMIT ?1"
    };

    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok(KnowledgeGap {
            id: row.get(0)?,
            page_id: row.get(1)?,
            query_hash: row.get(2)?,
            context: row.get(3)?,
            confidence_score: row.get(4)?,
            sensitivity: row.get(5)?,
            resolved_at: row.get(6)?,
            resolved_by_slug: row.get(7)?,
            detected_at: row.get(8)?,
        })
    })?;

    let mut gaps = Vec::new();
    for row in rows {
        gaps.push(row?);
    }
    Ok(gaps)
}

/// Mark a gap as resolved by linking it to the page that answered the query.
pub fn resolve_gap(id: i64, resolved_by_slug: &str, conn: &Connection) -> Result<(), GapsError> {
    let rows = conn.execute(
        "UPDATE knowledge_gaps SET \
             resolved_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), \
             resolved_by_slug = ?1 \
         WHERE id = ?2 AND resolved_at IS NULL",
        params![resolved_by_slug, id],
    )?;
    if rows == 0 {
        return Err(GapsError::NotFound { id });
    }
    Ok(())
}

fn sha256_hex(data: &str) -> String {
    let digest = Sha256::digest(data.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::core::db;

    fn open_test_db() -> Connection {
        db::open(":memory:").expect("open db")
    }

    #[test]
    fn log_gap_inserts_a_row() {
        let conn = open_test_db();
        log_gap(
            None,
            "who invented quantum socks",
            "query context",
            Some(0.1),
            &conn,
        )
        .unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM knowledge_gaps", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let sensitivity: String = conn
            .query_row(
                "SELECT sensitivity FROM knowledge_gaps LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sensitivity, "internal");
    }

    #[test]
    fn duplicate_query_is_idempotent() {
        let conn = open_test_db();
        log_gap(None, "same query twice", "", None, &conn).unwrap();
        log_gap(None, "same query twice", "", None, &conn).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM knowledge_gaps", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn list_gaps_returns_only_unresolved_by_default() {
        let conn = open_test_db();
        log_gap(None, "unresolved query", "", None, &conn).unwrap();
        log_gap(None, "resolved query", "", None, &conn).unwrap();

        // Resolve the second gap
        let id: i64 = conn
            .query_row(
                "SELECT id FROM knowledge_gaps WHERE query_hash = ?1",
                [sha256_hex("resolved query")],
                |row| row.get(0),
            )
            .unwrap();
        resolve_gap(id, "answers/quantum", &conn).unwrap();

        let unresolved = list_gaps(false, 100, &conn).unwrap();
        assert_eq!(unresolved.len(), 1);
        assert!(unresolved[0].resolved_at.is_none());

        let all = list_gaps(true, 100, &conn).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn resolve_gap_sets_resolved_at() {
        let conn = open_test_db();
        log_gap(None, "test query", "", None, &conn).unwrap();

        let id: i64 = conn
            .query_row("SELECT id FROM knowledge_gaps LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        resolve_gap(id, "people/alice", &conn).unwrap();

        let resolved_at: Option<String> = conn
            .query_row(
                "SELECT resolved_at FROM knowledge_gaps WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(resolved_at.is_some());

        let slug: String = conn
            .query_row(
                "SELECT resolved_by_slug FROM knowledge_gaps WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(slug, "people/alice");
    }

    #[test]
    fn list_gaps_with_resolved_true_includes_resolved_rows() {
        let conn = open_test_db();
        log_gap(None, "resolved query", "", None, &conn).unwrap();
        let id: i64 = conn
            .query_row("SELECT id FROM knowledge_gaps LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        resolve_gap(id, "people/alice", &conn).unwrap();

        let gaps = list_gaps(true, 10, &conn).unwrap();

        assert!(gaps[0].resolved_at.is_some());
    }

    #[test]
    fn list_gaps_preserves_page_binding_and_resolved_slug() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO pages
                 (collection_id, slug, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
             VALUES (1, 'notes/page-gap', 'note', 'Page Gap', '', '', '', '{}', '', '', 1)",
            [],
        )
        .unwrap();
        let page_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE slug = 'notes/page-gap'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        log_gap_for_page(page_id, "page bound query", "", None, &conn).unwrap();
        let gap_id: i64 = conn
            .query_row(
                "SELECT id FROM knowledge_gaps WHERE page_id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();
        resolve_gap(gap_id, "notes/page-gap", &conn).unwrap();

        let gaps = list_gaps(true, 10, &conn).unwrap();

        assert_eq!(gaps[0].page_id, Some(page_id));
        assert_eq!(gaps[0].resolved_by_slug.as_deref(), Some("notes/page-gap"));
    }

    #[test]
    fn resolve_gap_returns_error_for_unknown_id() {
        let conn = open_test_db();
        let result = resolve_gap(9999, "people/alice", &conn);
        assert!(result.is_err());
    }
}
