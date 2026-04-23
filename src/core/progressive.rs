use std::collections::HashSet;

use rusqlite::Connection;

use super::types::{SearchError, SearchResult};

/// Hard safety cap on expansion depth regardless of caller-supplied value.
const MAX_DEPTH: u32 = 3;

/// Expand an initial result set by following outbound links until the token
/// budget is exhausted or the depth cap is reached.
///
/// Token count is approximated as `len(compiled_truth) / 4`.
/// Results are deduplicated by slug. Initial results appear first, followed
/// by expansion results ordered by link distance.
pub fn progressive_retrieve(
    initial: Vec<SearchResult>,
    budget: usize,
    depth: u32,
    conn: &Connection,
) -> Result<Vec<SearchResult>, SearchError> {
    if initial.is_empty() || depth == 0 {
        return Ok(initial);
    }

    let effective_depth = depth.min(MAX_DEPTH);

    let mut seen: HashSet<String> = HashSet::new();
    let mut results: Vec<SearchResult> = Vec::new();
    let mut tokens_used: usize = 0;

    // Consume initial results, tracking budget
    for r in &initial {
        let cost = token_cost(&r.slug, conn);
        if tokens_used + cost > budget {
            break;
        }
        seen.insert(r.slug.clone());
        tokens_used += cost;
        results.push(r.clone());
    }

    // BFS expansion: each hop is one depth level
    let mut frontier: Vec<String> = results.iter().map(|r| r.slug.clone()).collect();

    for _hop in 0..effective_depth {
        if frontier.is_empty() || tokens_used >= budget {
            break;
        }

        let mut next_frontier: Vec<String> = Vec::new();

        for slug in &frontier {
            let neighbours = outbound_neighbours(slug, conn)?;
            for neighbour in neighbours {
                if !seen.insert(neighbour.slug.clone()) {
                    continue;
                }

                let cost = token_cost(&neighbour.slug, conn);
                if tokens_used + cost > budget {
                    continue;
                }

                tokens_used += cost;
                next_frontier.push(neighbour.slug.clone());
                results.push(neighbour);
            }
        }

        frontier = next_frontier;
    }

    Ok(results)
}

/// Approximate token cost of a page: `len(compiled_truth) / 4`.
fn token_cost(slug: &str, conn: &Connection) -> usize {
    conn.query_row(
        "SELECT LENGTH(compiled_truth) FROM pages WHERE slug = ?1",
        [slug],
        |row| row.get::<_, i64>(0),
    )
    .map(|len| (len as usize) / 4)
    .unwrap_or(0)
}

/// Fetch outbound link targets from a page, returning them as SearchResults.
fn outbound_neighbours(slug: &str, conn: &Connection) -> Result<Vec<SearchResult>, SearchError> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT p2.slug, p2.title, p2.summary, p2.wing \
             FROM links l \
             JOIN pages p1 ON l.from_page_id = p1.id \
             JOIN pages p2 ON l.to_page_id = p2.id \
             WHERE p1.slug = ?1 \
               AND (l.valid_from IS NULL OR l.valid_from <= date('now')) \
               AND (l.valid_until IS NULL OR l.valid_until >= date('now'))",
        )
        .map_err(SearchError::from)?;

    let rows = stmt
        .query_map([slug], |row| {
            Ok(SearchResult {
                slug: row.get(0)?,
                title: row.get(1)?,
                summary: row.get(2)?,
                score: 0.0,
                wing: row.get(3)?,
            })
        })
        .map_err(SearchError::from)?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row.map_err(SearchError::from)?);
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::core::db;

    fn open_test_db() -> Connection {
        db::open(":memory:").expect("open in-memory db")
    }

    fn insert_page(conn: &Connection, slug: &str, truth: &str) {
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version) \
             VALUES (?1, 'concept', ?1, ?1, ?2, '', '{}', '', '', 1)",
            rusqlite::params![slug, truth],
        )
        .unwrap();
    }

    fn insert_link(conn: &Connection, from: &str, to: &str) {
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
            "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind) \
             VALUES (?1, ?2, 'related', 'programmatic')",
            rusqlite::params![from_id, to_id],
        )
        .unwrap();
    }

    fn make_result(slug: &str) -> SearchResult {
        SearchResult {
            slug: slug.to_owned(),
            title: slug.to_owned(),
            summary: slug.to_owned(),
            score: 1.0,
            wing: "".to_owned(),
        }
    }

    // ── 5.6: empty initial returns empty ─────────────────────
    #[test]
    fn empty_initial_returns_empty() {
        let conn = open_test_db();
        let result = progressive_retrieve(vec![], 4000, 2, &conn).unwrap();
        assert!(result.is_empty());
    }

    // ── 5.6: zero depth returns initial results unchanged ────
    #[test]
    fn zero_depth_returns_initial_unchanged() {
        let conn = open_test_db();
        // 100 chars = 25 tokens
        insert_page(&conn, "a", &"x".repeat(100));
        insert_page(&conn, "b", &"y".repeat(100));
        insert_link(&conn, "a", "b");

        let initial = vec![make_result("a")];
        let result = progressive_retrieve(initial.clone(), 100_000, 0, &conn).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].slug, "a");
    }

    // ── 5.6: budget exhausted before depth cap ───────────────
    #[test]
    fn budget_exhausted_stops_expansion() {
        let conn = open_test_db();
        // a: 400 chars = 100 tokens, b: 400 chars = 100 tokens, c: 400 chars = 100 tokens
        insert_page(&conn, "a", &"x".repeat(400));
        insert_page(&conn, "b", &"y".repeat(400));
        insert_page(&conn, "c", &"z".repeat(400));
        insert_link(&conn, "a", "b");
        insert_link(&conn, "b", "c");

        // Budget = 150 tokens: a (100) fits, b (100) would exceed 200 > 150
        let initial = vec![make_result("a")];
        let result = progressive_retrieve(initial, 150, 3, &conn).unwrap();

        assert_eq!(result.len(), 1, "b should not fit in the budget");
        assert_eq!(result[0].slug, "a");
    }

    // ── 5.6: depth cap stops expansion before budget ─────────
    #[test]
    fn depth_cap_stops_expansion() {
        let conn = open_test_db();
        insert_page(&conn, "a", &"x".repeat(40));
        insert_page(&conn, "b", &"y".repeat(40));
        insert_page(&conn, "c", &"z".repeat(40));
        insert_link(&conn, "a", "b");
        insert_link(&conn, "b", "c");

        // depth=1 with huge budget: should get a + b but NOT c
        let initial = vec![make_result("a")];
        let result = progressive_retrieve(initial, 100_000, 1, &conn).unwrap();
        let slugs: HashSet<&str> = result.iter().map(|r| r.slug.as_str()).collect();

        assert!(slugs.contains("a"));
        assert!(slugs.contains("b"));
        assert!(!slugs.contains("c"), "depth=1 should not reach second hop");
    }

    // ── 5.6: duplicates from expansion are deduplicated ──────
    #[test]
    fn duplicates_are_deduplicated() {
        let conn = open_test_db();
        insert_page(&conn, "a", &"x".repeat(40));
        insert_page(&conn, "b", &"y".repeat(40));
        insert_page(&conn, "shared", &"z".repeat(40));
        insert_link(&conn, "a", "shared");
        insert_link(&conn, "b", "shared");

        let initial = vec![make_result("a"), make_result("b")];
        let result = progressive_retrieve(initial, 100_000, 1, &conn).unwrap();

        let shared_count = result.iter().filter(|r| r.slug == "shared").count();
        assert_eq!(shared_count, 1, "shared page should appear exactly once");
    }

    // ── Additional: multi-hop expansion works ────────────────
    #[test]
    fn multi_hop_expansion_reaches_depth_2() {
        let conn = open_test_db();
        insert_page(&conn, "a", &"x".repeat(40));
        insert_page(&conn, "b", &"y".repeat(40));
        insert_page(&conn, "c", &"z".repeat(40));
        insert_link(&conn, "a", "b");
        insert_link(&conn, "b", "c");

        let initial = vec![make_result("a")];
        let result = progressive_retrieve(initial, 100_000, 2, &conn).unwrap();
        let slugs: HashSet<&str> = result.iter().map(|r| r.slug.as_str()).collect();

        assert!(slugs.contains("a"));
        assert!(slugs.contains("b"));
        assert!(slugs.contains("c"));
    }

    // ── Hard cap at MAX_DEPTH (3) ────────────────────────────
    #[test]
    fn depth_is_capped_at_3() {
        let conn = open_test_db();
        insert_page(&conn, "a", &"x".repeat(40));
        insert_page(&conn, "b", &"y".repeat(40));
        insert_page(&conn, "c", &"z".repeat(40));
        insert_page(&conn, "d", &"w".repeat(40));
        insert_page(&conn, "e", &"v".repeat(40));
        insert_link(&conn, "a", "b");
        insert_link(&conn, "b", "c");
        insert_link(&conn, "c", "d");
        insert_link(&conn, "d", "e");

        // Request depth 10 — capped at 3, so e (4 hops away) should not appear
        let initial = vec![make_result("a")];
        let result = progressive_retrieve(initial, 100_000, 10, &conn).unwrap();
        let slugs: HashSet<&str> = result.iter().map(|r| r.slug.as_str()).collect();

        assert!(slugs.contains("d"), "d is 3 hops away, should appear");
        assert!(!slugs.contains("e"), "e is 4 hops away, capped at 3");
    }
}
