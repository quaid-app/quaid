use std::collections::HashSet;

use rusqlite::Connection;

use super::collections::{self, OpKind, SlugResolution};
use super::pages::{self, PageKey};
use super::search::{
    configured_max_chunks_per_doc, configured_relevance_floor, dedup_chunks_per_page,
    filter_below_floor,
};
use super::types::{SearchError, SearchResult};

/// Hard safety cap on expansion depth regardless of caller-supplied value.
const MAX_DEPTH: u32 = 3;

/// Expand an initial result set by following outbound links until the token
/// budget is exhausted or the depth cap is reached.
///
/// Token count is approximated as `(len(compiled_truth) + len(timeline)) / 4`.
/// Results are deduplicated by slug. Initial results appear first, followed
/// by expansion results ordered by link distance. The post-fusion quality
/// passes from `core::search` (per-page dedup, relevance floor) are applied
/// to the initial set and re-applied on every expansion step; both are
/// identity no-ops at their seeded config defaults.
///
/// `collection_filter` restricts expansion to pages belonging to the given
/// collection ID. Pass `None` to allow cross-collection expansion (CLI path).
pub fn progressive_retrieve(
    initial: Vec<SearchResult>,
    budget: usize,
    depth: u32,
    collection_filter: Option<i64>,
    include_superseded: bool,
    conn: &Connection,
) -> Result<Vec<SearchResult>, SearchError> {
    progressive_retrieve_with_namespace(
        initial,
        budget,
        depth,
        collection_filter,
        None,
        include_superseded,
        conn,
    )
}

/// Namespace-aware variant of [`progressive_retrieve`].
pub fn progressive_retrieve_with_namespace(
    initial: Vec<SearchResult>,
    budget: usize,
    depth: u32,
    collection_filter: Option<i64>,
    namespace_filter: Option<&str>,
    include_superseded: bool,
    conn: &Connection,
) -> Result<Vec<SearchResult>, SearchError> {
    let initial = if include_superseded {
        initial
    } else {
        initial
            .into_iter()
            .filter(|result| page_is_head(&result.slug, namespace_filter, conn))
            .collect()
    };

    // Apply the same post-fusion quality passes as `hybrid_search` (dedup →
    // floor) so token-budget expansion sees the same filtered candidate set
    // as the top-k API. Both passes are identity no-ops at their seeded
    // defaults. Below-floor candidates are never expanded.
    let max_chunks_per_doc = configured_max_chunks_per_doc(conn)?;
    let relevance_floor = configured_relevance_floor(conn)?;
    let initial = filter_below_floor(
        dedup_chunks_per_page(initial, max_chunks_per_doc),
        relevance_floor,
    );
    if initial.is_empty() || depth == 0 {
        return Ok(initial);
    }

    let effective_depth = depth.min(MAX_DEPTH);

    let mut seen: HashSet<String> = HashSet::new();
    let mut results: Vec<SearchResult> = Vec::new();
    let mut tokens_used: usize = 0;

    // Consume initial results, tracking budget. Entries that do not fit are
    // skipped (not `break`) so one oversized page cannot starve later
    // results — the same policy the expansion loop below applies. Entries
    // whose slug cannot be costed are skipped rather than riding for free.
    for r in &initial {
        let Some(cost) = token_cost(&r.slug, namespace_filter, conn) else {
            continue;
        };
        if tokens_used + cost > budget {
            continue;
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

        let mut hop_candidates: Vec<SearchResult> = Vec::new();
        for slug in &frontier {
            hop_candidates.extend(outbound_neighbours(
                slug,
                collection_filter,
                namespace_filter,
                include_superseded,
                conn,
            )?);
        }
        // Re-apply dedup + floor on every expansion step (identity at the
        // seeded defaults) so each hop honours the same quality bar as the
        // initial candidate set.
        let hop_candidates = filter_below_floor(
            dedup_chunks_per_page(hop_candidates, max_chunks_per_doc),
            relevance_floor,
        );

        let mut next_frontier: Vec<String> = Vec::new();
        for neighbour in hop_candidates {
            if !seen.insert(neighbour.slug.clone()) {
                continue;
            }

            let Some(cost) = token_cost(&neighbour.slug, namespace_filter, conn) else {
                continue;
            };
            if tokens_used + cost > budget {
                continue;
            }

            tokens_used += cost;
            next_frontier.push(neighbour.slug.clone());
            results.push(neighbour);
        }

        frontier = next_frontier;
    }

    Ok(results)
}

/// Approximate token cost of a page: `(len(compiled_truth) + len(timeline)) / 4`.
///
/// Returns `None` when the slug cannot be resolved or the page row is
/// missing, so callers can skip the entry instead of treating it as free.
fn token_cost(slug: &str, namespace_filter: Option<&str>, conn: &Connection) -> Option<usize> {
    let page_id = resolve_page_id(conn, slug, namespace_filter)?;

    conn.query_row(
        "SELECT LENGTH(compiled_truth) + LENGTH(timeline) FROM pages WHERE id = ?1",
        [page_id],
        |row| row.get::<_, i64>(0),
    )
    .ok()
    .map(|len| (len as usize) / 4)
}

fn page_is_head(slug: &str, namespace_filter: Option<&str>, conn: &Connection) -> bool {
    let Some(page_id) = resolve_page_id(conn, slug, namespace_filter) else {
        return false;
    };

    conn.query_row(
        "SELECT superseded_by IS NULL FROM pages WHERE id = ?1",
        [page_id],
        |row| row.get::<_, bool>(0),
    )
    .unwrap_or(false)
}

fn resolve_page_id(conn: &Connection, slug: &str, namespace_filter: Option<&str>) -> Option<i64> {
    let (collection_id, resolved_slug) = resolve_slug_key(conn, slug)?;
    pages::resolve_optional(
        conn,
        &PageKey {
            collection_id,
            namespace: namespace_filter,
            slug: &resolved_slug,
        },
    )
    .ok()
    .flatten()
}

/// Fetch outbound link targets from a page, returning them as SearchResults.
///
/// When `collection_filter` is `Some(id)`, only target pages belonging to that
/// collection are returned, enforcing the MCP read-filter contract during
/// `depth="auto"` expansion.
///
/// Quarantined targets are always excluded — retrieval never surfaces
/// quarantined pages; they remain reachable only via explicit access paths
/// such as `memory_get` and raw imports.
fn outbound_neighbours(
    slug: &str,
    collection_filter: Option<i64>,
    namespace_filter: Option<&str>,
    include_superseded: bool,
    conn: &Connection,
) -> Result<Vec<SearchResult>, SearchError> {
    let Some((collection_id, resolved_slug)) = resolve_slug_key(conn, slug) else {
        return Ok(Vec::new());
    };
    let canonical_slug = slug.contains("::");
    let target_slug_expr = if canonical_slug {
        "c2.name || '::' || p2.slug"
    } else {
        "p2.slug"
    };
    let collection_join = if canonical_slug {
        " JOIN collections c2 ON c2.id = p2.collection_id"
    } else {
        ""
    };
    // `?3 IS NULL` short-circuits when no collection filter is active (CLI path).
    let sql = format!(
        "SELECT {target_slug_expr}, p2.title, p2.summary, p2.wing \
         FROM links l \
         JOIN pages p1 ON l.from_page_id = p1.id \
         JOIN pages p2 ON l.to_page_id = p2.id{collection_join} \
         WHERE p1.collection_id = ?1 AND p1.slug = ?2 \
           AND p2.quarantined_at IS NULL \
           AND (l.valid_from IS NULL OR l.valid_from <= date('now')) \
           AND (l.valid_until IS NULL OR l.valid_until >= date('now')) \
           AND (?3 IS NULL OR p2.collection_id = ?3) \
           AND (?4 IS NULL \
                OR (?4 = '' AND p1.namespace = '') \
                OR (?4 != '' AND (p1.namespace = ?4 \
                    OR (p1.namespace = '' AND NOT EXISTS ( \
                        SELECT 1 FROM pages p1_ns \
                        WHERE p1_ns.collection_id = p1.collection_id \
                          AND p1_ns.slug = p1.slug \
                          AND p1_ns.namespace = ?4))))) \
            AND (?4 IS NULL \
                 OR (?4 = '' AND p2.namespace = '') \
                 OR (?4 != '' AND (p2.namespace = ?4 OR p2.namespace = ''))){}",
        if include_superseded {
            ""
        } else {
            " AND p2.superseded_by IS NULL"
        }
    );
    let mut stmt = conn.prepare_cached(&sql).map_err(SearchError::from)?;

    let rows = stmt
        .query_map(
            rusqlite::params![
                collection_id,
                resolved_slug,
                collection_filter,
                namespace_filter
            ],
            |row| {
                Ok(SearchResult {
                    slug: row.get(0)?,
                    title: row.get(1)?,
                    summary: row.get(2)?,
                    score: 0.0,
                    wing: row.get(3)?,
                    ..Default::default()
                })
            },
        )
        .map_err(SearchError::from)?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row.map_err(SearchError::from)?);
    }
    Ok(results)
}

fn resolve_slug_key(conn: &Connection, slug: &str) -> Option<(i64, String)> {
    match collections::parse_slug(conn, slug, OpKind::Read).ok()? {
        SlugResolution::Resolved {
            collection_id,
            slug,
            ..
        } => Some((collection_id, slug)),
        SlugResolution::NotFound { .. } | SlugResolution::Ambiguous { .. } => None,
    }
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

    fn supersede_page(conn: &Connection, predecessor_slug: &str, successor_slug: &str) {
        let successor_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE slug = ?1",
                [successor_slug],
                |row| row.get(0),
            )
            .unwrap();
        conn.execute(
            "UPDATE pages SET superseded_by = ?1 WHERE slug = ?2",
            rusqlite::params![successor_id, predecessor_slug],
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
            ..Default::default()
        }
    }

    // ── 5.6: empty initial returns empty ─────────────────────
    #[test]
    fn empty_initial_returns_empty() {
        let conn = open_test_db();
        let result = progressive_retrieve(vec![], 4000, 2, None, false, &conn).unwrap();
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
        let result = progressive_retrieve(initial, 100_000, 0, None, false, &conn).unwrap();

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
        let result = progressive_retrieve(initial, 150, 3, None, false, &conn).unwrap();

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
        let result = progressive_retrieve(initial, 100_000, 1, None, false, &conn).unwrap();
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
        let result = progressive_retrieve(initial, 100_000, 1, None, false, &conn).unwrap();

        let shared_count = result.iter().filter(|r| r.slug == "shared").count();
        assert_eq!(shared_count, 1, "shared page should appear exactly once");
    }

    #[test]
    fn expansion_skips_superseded_neighbours_unless_requested() {
        let conn = open_test_db();
        insert_page(&conn, "a", &"x".repeat(40));
        insert_page(&conn, "b-old", &"y".repeat(40));
        insert_page(&conn, "b-new", &"z".repeat(40));
        insert_link(&conn, "a", "b-old");
        supersede_page(&conn, "b-old", "b-new");

        let default_result =
            progressive_retrieve(vec![make_result("a")], 100_000, 1, None, false, &conn).unwrap();
        let historical_result =
            progressive_retrieve(vec![make_result("a")], 100_000, 1, None, true, &conn).unwrap();

        assert_eq!(
            default_result
                .iter()
                .map(|result| result.slug.as_str())
                .collect::<Vec<_>>(),
            vec!["a"]
        );
        assert_eq!(
            historical_result
                .iter()
                .map(|result| result.slug.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b-old"]
        );
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
        let result = progressive_retrieve(initial, 100_000, 2, None, false, &conn).unwrap();
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
        let result = progressive_retrieve(initial, 100_000, 10, None, false, &conn).unwrap();
        let slugs: HashSet<&str> = result.iter().map(|r| r.slug.as_str()).collect();

        assert!(slugs.contains("d"), "d is 3 hops away, should appear");
        assert!(!slugs.contains("e"), "e is 4 hops away, capped at 3");
    }
}
