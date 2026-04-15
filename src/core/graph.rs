use std::collections::{HashSet, VecDeque};

use rusqlite::Connection;
use serde::Serialize;
use thiserror::Error;

// ── Types ────────────────────────────────────────────────────

/// Controls whether the graph traversal includes closed (past) links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemporalFilter {
    /// Only links that are currently active (valid_until IS NULL or >= today).
    Active,
    /// All links regardless of temporal validity.
    All,
}

/// A node in the neighbourhood graph result.
#[derive(Debug, Clone, Serialize)]
pub struct GraphNode {
    pub slug: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub title: String,
}

/// An edge in the neighbourhood graph result.
#[derive(Debug, Clone, Serialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub relationship: String,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}

/// The complete result of a neighbourhood graph query.
#[derive(Debug, Clone, Serialize)]
pub struct GraphResult {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// Errors that can occur during graph traversal.
#[derive(Debug, Error)]
pub enum GraphError {
    #[error("page not found: {slug}")]
    PageNotFound { slug: String },

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

// ── Constants ────────────────────────────────────────────────

/// Hard safety cap on traversal depth regardless of caller-supplied argument.
const MAX_DEPTH: u32 = 10;

// ── Core BFS ─────────────────────────────────────────────────

/// Perform an N-hop BFS over the `links` table starting from `slug`.
///
/// Returns a deduplicated set of reachable nodes and the edges connecting them.
/// Depth is capped at [`MAX_DEPTH`] (10) regardless of the caller-supplied value.
/// A `HashSet<i64>` visited set prevents cycles from causing infinite loops.
pub fn neighborhood_graph(
    slug: &str,
    depth: u32,
    filter: TemporalFilter,
    conn: &Connection,
) -> Result<GraphResult, GraphError> {
    let effective_depth = depth.min(MAX_DEPTH);

    // Resolve root slug → page id
    let root: (i64, String, String) = conn
        .query_row(
            "SELECT id, type, title FROM pages WHERE slug = ?1",
            [slug],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => GraphError::PageNotFound {
                slug: slug.to_string(),
            },
            other => GraphError::Sqlite(other),
        })?;

    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut visited: HashSet<i64> = HashSet::new();

    // Seed BFS with root
    visited.insert(root.0);
    nodes.push(GraphNode {
        slug: slug.to_string(),
        node_type: root.1,
        title: root.2,
    });

    if effective_depth == 0 {
        return Ok(GraphResult { nodes, edges });
    }

    // BFS frontier: (page_id, current_depth)
    let mut queue: VecDeque<(i64, u32)> = VecDeque::new();
    queue.push_back((root.0, 0));

    // Build the temporal clause: active links must have started and not yet ended.
    let temporal_clause = match filter {
        TemporalFilter::Active => {
            " AND (l.valid_from IS NULL OR l.valid_from <= date('now'))\
             AND (l.valid_until IS NULL OR l.valid_until >= date('now'))"
        }
        TemporalFilter::All => "",
    };

    // Outbound-only BFS: a page's neighbourhood is the set of pages it explicitly
    // links to.  Inbound links are accessible via `gbrain backlinks`, keeping the
    // two directions orthogonal and the rendering unambiguous.
    let outbound_sql = format!(
        "SELECT l.id, l.to_page_id, p.slug, p.type, p.title, \
                l.relationship, l.valid_from, l.valid_until \
         FROM links l \
         JOIN pages p ON l.to_page_id = p.id \
         WHERE l.from_page_id = ?1{temporal_clause}"
    );

    let mut seen_edges: HashSet<i64> = HashSet::new();

    while let Some((page_id, current_depth)) = queue.pop_front() {
        if current_depth >= effective_depth {
            continue;
        }

        // Resolve the current node's slug for edge recording.
        let current_slug: String =
            conn.query_row("SELECT slug FROM pages WHERE id = ?1", [page_id], |row| {
                row.get(0)
            })?;

        let mut stmt = conn.prepare_cached(&outbound_sql)?;
        let rows = stmt.query_map([page_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })?;

        for row in rows {
            let (link_id, target_id, to_slug, to_type, to_title, rel, vf, vu) = row?;

            if seen_edges.insert(link_id) {
                edges.push(GraphEdge {
                    from: current_slug.clone(),
                    to: to_slug.clone(),
                    relationship: rel,
                    valid_from: vf,
                    valid_until: vu,
                });
            }

            if visited.insert(target_id) {
                nodes.push(GraphNode {
                    slug: to_slug,
                    node_type: to_type,
                    title: to_title,
                });
                queue.push_back((target_id, current_depth + 1));
            }
        }
    }

    Ok(GraphResult { nodes, edges })
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;

    fn open_test_db() -> Connection {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        std::mem::forget(dir);
        conn
    }

    fn insert_page(conn: &Connection, slug: &str, page_type: &str, title: &str) {
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version) \
             VALUES (?1, ?2, ?3, '', '', '', '{}', '', '', 1)",
            rusqlite::params![slug, page_type, title],
        )
        .unwrap();
    }

    fn insert_link(
        conn: &Connection,
        from: &str,
        to: &str,
        rel: &str,
        valid_from: Option<&str>,
        valid_until: Option<&str>,
    ) {
        let from_id: i64 = conn
            .query_row("SELECT id FROM pages WHERE slug = ?1", [from], |row| {
                row.get(0)
            })
            .unwrap();
        let to_id: i64 = conn
            .query_row("SELECT id FROM pages WHERE slug = ?1", [to], |row| {
                row.get(0)
            })
            .unwrap();
        conn.execute(
            "INSERT INTO links (from_page_id, to_page_id, relationship, valid_from, valid_until) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![from_id, to_id, rel, valid_from, valid_until],
        )
        .unwrap();
    }

    // ── Task 1.5: zero-hop returns root only ─────────────────

    #[test]
    fn zero_hop_returns_root_only() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person", "Alice");
        insert_page(&conn, "companies/acme", "company", "Acme");
        insert_link(
            &conn,
            "people/alice",
            "companies/acme",
            "works_at",
            None,
            None,
        );

        let result = neighborhood_graph("people/alice", 0, TemporalFilter::Active, &conn).unwrap();

        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].slug, "people/alice");
        assert!(result.edges.is_empty());
    }

    // ── Task 1.5: single-hop returns direct neighbours ───────

    #[test]
    fn single_hop_returns_direct_neighbours() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person", "Alice");
        insert_page(&conn, "companies/acme", "company", "Acme");
        insert_page(&conn, "companies/beta", "company", "Beta");
        insert_link(
            &conn,
            "people/alice",
            "companies/acme",
            "works_at",
            None,
            None,
        );
        insert_link(
            &conn,
            "people/alice",
            "companies/beta",
            "advises",
            None,
            None,
        );

        let result = neighborhood_graph("people/alice", 1, TemporalFilter::Active, &conn).unwrap();

        assert_eq!(result.nodes.len(), 3);
        let slugs: HashSet<&str> = result.nodes.iter().map(|n| n.slug.as_str()).collect();
        assert!(slugs.contains("people/alice"));
        assert!(slugs.contains("companies/acme"));
        assert!(slugs.contains("companies/beta"));
        assert_eq!(result.edges.len(), 2);
    }

    // ── Task 1.5: cycle terminates without infinite loop ─────

    #[test]
    fn cycle_between_two_pages_terminates() {
        let conn = open_test_db();
        insert_page(&conn, "a", "concept", "A");
        insert_page(&conn, "b", "concept", "B");
        insert_link(&conn, "a", "b", "related", None, None);
        insert_link(&conn, "b", "a", "related", None, None);

        let result = neighborhood_graph("a", 10, TemporalFilter::All, &conn).unwrap();

        let slugs: HashSet<&str> = result.nodes.iter().map(|n| n.slug.as_str()).collect();
        assert_eq!(slugs.len(), 2);
        assert!(slugs.contains("a"));
        assert!(slugs.contains("b"));
    }

    // ── Task 1.5: temporal filter excludes past-closed links ─

    #[test]
    fn temporal_active_excludes_past_closed_links() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person", "Alice");
        insert_page(&conn, "companies/acme", "company", "Acme");
        insert_link(
            &conn,
            "people/alice",
            "companies/acme",
            "works_at",
            Some("2020-01-01"),
            Some("2020-12-31"),
        );

        let result = neighborhood_graph("people/alice", 1, TemporalFilter::Active, &conn).unwrap();

        assert_eq!(result.nodes.len(), 1, "closed link should be excluded");
        assert_eq!(result.nodes[0].slug, "people/alice");
        assert!(result.edges.is_empty());
    }

    // ── Task 1.5: All filter includes past-closed links ──────

    #[test]
    fn temporal_all_includes_past_closed_links() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person", "Alice");
        insert_page(&conn, "companies/acme", "company", "Acme");
        insert_link(
            &conn,
            "people/alice",
            "companies/acme",
            "works_at",
            Some("2020-01-01"),
            Some("2020-12-31"),
        );

        let result = neighborhood_graph("people/alice", 1, TemporalFilter::All, &conn).unwrap();

        assert_eq!(result.nodes.len(), 2);
        let slugs: HashSet<&str> = result.nodes.iter().map(|n| n.slug.as_str()).collect();
        assert!(slugs.contains("companies/acme"));
    }

    // ── Task 1.5: unknown slug returns PageNotFound ──────────

    #[test]
    fn unknown_slug_returns_page_not_found() {
        let conn = open_test_db();

        let result = neighborhood_graph("people/ghost", 1, TemporalFilter::Active, &conn);

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            GraphError::PageNotFound { .. }
        ));
    }

    // ── Additional: multi-hop traversal ──────────────────────

    #[test]
    fn two_hop_traversal_reaches_indirect_neighbours() {
        let conn = open_test_db();
        insert_page(&conn, "a", "concept", "A");
        insert_page(&conn, "b", "concept", "B");
        insert_page(&conn, "c", "concept", "C");
        insert_link(&conn, "a", "b", "related", None, None);
        insert_link(&conn, "b", "c", "related", None, None);

        // depth=1 should NOT reach C
        let r1 = neighborhood_graph("a", 1, TemporalFilter::Active, &conn).unwrap();
        let slugs1: HashSet<&str> = r1.nodes.iter().map(|n| n.slug.as_str()).collect();
        assert!(!slugs1.contains("c"));

        // depth=2 should reach C
        let r2 = neighborhood_graph("a", 2, TemporalFilter::Active, &conn).unwrap();
        let slugs2: HashSet<&str> = r2.nodes.iter().map(|n| n.slug.as_str()).collect();
        assert!(slugs2.contains("c"));
    }

    // ── Depth cap at 10 ──────────────────────────────────────

    #[test]
    fn depth_is_capped_at_max_depth() {
        let conn = open_test_db();
        insert_page(&conn, "root", "concept", "Root");

        // Request depth 999 — should not panic, effectively capped at MAX_DEPTH
        let result = neighborhood_graph("root", 999, TemporalFilter::Active, &conn).unwrap();
        assert_eq!(result.nodes.len(), 1);
    }

    // ── Temporal filter respects valid_from (future links not active) ──

    #[test]
    fn temporal_active_excludes_future_links() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person", "Alice");
        insert_page(&conn, "companies/acme", "company", "Acme");
        // Link starting far in the future
        insert_link(
            &conn,
            "people/alice",
            "companies/acme",
            "works_at",
            Some("2099-01-01"),
            None,
        );

        let result = neighborhood_graph("people/alice", 1, TemporalFilter::Active, &conn).unwrap();

        assert_eq!(
            result.nodes.len(),
            1,
            "future-dated link should be excluded from active graph"
        );
        assert!(result.edges.is_empty());
    }

    // ── Graph node fields are correct ────────────────────────

    #[test]
    fn graph_node_has_correct_type_and_title() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person", "Alice Johnson");

        let result = neighborhood_graph("people/alice", 0, TemporalFilter::Active, &conn).unwrap();

        assert_eq!(result.nodes[0].node_type, "person");
        assert_eq!(result.nodes[0].title, "Alice Johnson");
    }

    // ── Edge fields are correct ──────────────────────────────

    #[test]
    fn graph_edge_has_correct_fields() {
        let conn = open_test_db();
        insert_page(&conn, "people/alice", "person", "Alice");
        insert_page(&conn, "companies/acme", "company", "Acme");
        insert_link(
            &conn,
            "people/alice",
            "companies/acme",
            "works_at",
            Some("2024-01"),
            None,
        );

        let result = neighborhood_graph("people/alice", 1, TemporalFilter::Active, &conn).unwrap();

        assert_eq!(result.edges.len(), 1);
        assert_eq!(result.edges[0].from, "people/alice");
        assert_eq!(result.edges[0].to, "companies/acme");
        assert_eq!(result.edges[0].relationship, "works_at");
        assert_eq!(result.edges[0].valid_from.as_deref(), Some("2024-01"));
        assert!(result.edges[0].valid_until.is_none());
    }
}
