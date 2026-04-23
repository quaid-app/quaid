use std::collections::HashMap;

use rusqlite::Connection;

use super::fts::{sanitize_fts_query, search_fts};
use super::inference::search_vec;
use super::types::{SearchError, SearchMergeStrategy, SearchResult};

/// Hybrid search with exact-slug short-circuit, FTS5, and vector search.
///
/// At most `limit` results are returned. The limit is pushed into the FTS5 query
/// and applied after the merge step to cap memory usage.
pub fn hybrid_search(
    query: &str,
    wing: Option<&str>,
    conn: &Connection,
    limit: usize,
) -> Result<Vec<SearchResult>, SearchError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    if let Some(slug) = exact_slug_query(trimmed) {
        if let Some(result) = exact_slug_result(slug, wing, conn)? {
            return Ok(vec![result]);
        }
    }

    let fts_safe = sanitize_fts_query(trimmed);
    if !has_natural_language_terms(&fts_safe) {
        return Ok(Vec::new());
    }

    let fts_results = search_fts(&fts_safe, wing, conn, limit)?;
    let vec_results = search_vec(trimmed, 10, wing, conn)?;

    let mut merged = match read_merge_strategy(conn)? {
        SearchMergeStrategy::SetUnion => merge_set_union(&fts_results, &vec_results),
        SearchMergeStrategy::Rrf => merge_rrf(&fts_results, &vec_results),
    };
    merged.truncate(limit);
    Ok(merged)
}

fn has_natural_language_terms(fts_safe: &str) -> bool {
    const QUOTED_FTS5_KEYWORDS: &[&str] = &["\"AND\"", "\"OR\"", "\"NOT\"", "\"NEAR\""];

    fts_safe
        .split_whitespace()
        .any(|token| !QUOTED_FTS5_KEYWORDS.contains(&token))
}

/// Reads the configured hybrid-search merge strategy.
pub fn read_merge_strategy(conn: &Connection) -> Result<SearchMergeStrategy, SearchError> {
    let value = conn.query_row(
        "SELECT value FROM config WHERE key = 'search_merge_strategy'",
        [],
        |row| row.get::<_, String>(0),
    );

    match value {
        Ok(value) => Ok(SearchMergeStrategy::from_config(&value)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(SearchMergeStrategy::SetUnion),
        Err(err) => Err(SearchError::from(err)),
    }
}

fn exact_slug_query(query: &str) -> Option<&str> {
    let stripped = query
        .strip_prefix("[[")
        .and_then(|value| value.strip_suffix("]]"))
        .unwrap_or(query);
    let trimmed = stripped.trim();

    if trimmed.is_empty() || trimmed.contains(char::is_whitespace) {
        None
    } else {
        Some(trimmed)
    }
}

fn exact_slug_result(
    slug: &str,
    wing: Option<&str>,
    conn: &Connection,
) -> Result<Option<SearchResult>, SearchError> {
    let query = if wing.is_some() {
        "SELECT slug, title, summary, wing FROM pages WHERE slug = ?1 AND wing = ?2 LIMIT 1"
    } else {
        "SELECT slug, title, summary, wing FROM pages WHERE slug = ?1 LIMIT 1"
    };

    let result = if let Some(wing) = wing {
        conn.query_row(query, rusqlite::params![slug, wing], |row| {
            Ok(SearchResult {
                slug: row.get(0)?,
                title: row.get(1)?,
                summary: row.get(2)?,
                score: 1.0,
                wing: row.get(3)?,
            })
        })
    } else {
        conn.query_row(query, [slug], |row| {
            Ok(SearchResult {
                slug: row.get(0)?,
                title: row.get(1)?,
                summary: row.get(2)?,
                score: 1.0,
                wing: row.get(3)?,
            })
        })
    };

    match result {
        Ok(result) => Ok(Some(result)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) => Err(SearchError::from(err)),
    }
}

fn merge_set_union(
    fts_results: &[SearchResult],
    vec_results: &[SearchResult],
) -> Vec<SearchResult> {
    let mut merged: HashMap<String, SearchResult> = HashMap::new();
    let fts_max = max_score(fts_results);
    let vec_max = max_score(vec_results);

    for result in fts_results {
        let normalized = normalize_score(result.score, fts_max);
        merged.insert(
            result.slug.clone(),
            SearchResult {
                score: normalized * 0.4,
                ..result.clone()
            },
        );
    }

    for result in vec_results {
        let normalized = normalize_score(result.score, vec_max) * 0.6;
        merged
            .entry(result.slug.clone())
            .and_modify(|existing| existing.score += normalized)
            .or_insert_with(|| SearchResult {
                score: normalized,
                ..result.clone()
            });
    }

    let mut results: Vec<_> = merged.into_values().collect();
    results.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.slug.cmp(&right.slug))
    });
    results
}

fn merge_rrf(fts_results: &[SearchResult], vec_results: &[SearchResult]) -> Vec<SearchResult> {
    const RRF_K: f64 = 60.0;

    let mut merged: HashMap<String, SearchResult> = HashMap::new();

    for (rank, result) in fts_results.iter().enumerate() {
        let contribution = 1.0 / (RRF_K + rank as f64 + 1.0);
        merged.insert(
            result.slug.clone(),
            SearchResult {
                score: contribution,
                ..result.clone()
            },
        );
    }

    for (rank, result) in vec_results.iter().enumerate() {
        let contribution = 1.0 / (RRF_K + rank as f64 + 1.0);
        merged
            .entry(result.slug.clone())
            .and_modify(|existing| existing.score += contribution)
            .or_insert_with(|| SearchResult {
                score: contribution,
                ..result.clone()
            });
    }

    let mut results: Vec<_> = merged.into_values().collect();
    results.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.slug.cmp(&right.slug))
    });
    results
}

fn max_score(results: &[SearchResult]) -> f64 {
    results
        .iter()
        .map(|result| result.score)
        .fold(0.0_f64, f64::max)
}

fn normalize_score(score: f64, max_score: f64) -> f64 {
    if max_score > 0.0 {
        score / max_score
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;
    use crate::commands::embed;
    use crate::core::db;

    fn open_test_db() -> Connection {
        let dir = tempfile::TempDir::new().expect("create temp dir");
        let db_path = dir.path().join("test_brain.db");
        let conn = db::open(db_path.to_str().expect("utf8 path")).expect("open db");
        std::mem::forget(dir);
        conn
    }

    fn result(slug: &str, score: f64) -> SearchResult {
        SearchResult {
            slug: slug.to_owned(),
            title: slug.to_owned(),
            summary: format!("summary for {slug}"),
            score,
            wing: "people".to_owned(),
        }
    }

    fn insert_page(
        conn: &Connection,
        slug: &str,
        title: &str,
        summary: &str,
        truth: &str,
        wing: &str,
    ) {
        let mut hex = String::new();
        for byte in slug.as_bytes() {
            hex.push_str(&format!("{byte:02x}"));
            if hex.len() >= 32 {
                break;
            }
        }
        while hex.len() < 32 {
            hex.push('0');
        }
        let uuid = format!(
            "{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        );

        conn.execute(
            "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version) \
             VALUES (?1, ?2, 'person', ?3, ?4, ?5, '', '{}', ?6, '', 1)",
            rusqlite::params![slug, uuid, title, summary, truth, wing],
        )
        .expect("insert page");
    }

    #[test]
    fn hybrid_search_short_circuits_exact_slug_queries() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "people/alice",
            "Alice",
            "Founder",
            "Alice works on AI agents.",
            "people",
        );

        let results = hybrid_search("people/alice", None, &conn, 1000).expect("hybrid search");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].slug, "people/alice");
    }

    #[test]
    fn hybrid_search_short_circuits_wikilink_queries() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "people/alice",
            "Alice",
            "Founder",
            "Alice works on AI agents.",
            "people",
        );

        let results = hybrid_search("[[people/alice]]", None, &conn, 1000).expect("hybrid search");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].slug, "people/alice");
    }

    #[test]
    fn merge_set_union_returns_unique_results() {
        let fts = vec![result("a", 10.0), result("b", 9.0), result("c", 8.0)];
        let vec = vec![result("b", 8.0), result("c", 7.0), result("d", 6.0)];

        let results = merge_set_union(&fts, &vec);
        let slugs: Vec<_> = results.iter().map(|result| result.slug.as_str()).collect();

        assert_eq!(slugs.len(), 4);
        assert!(slugs.contains(&"a"));
        assert!(slugs.contains(&"b"));
        assert!(slugs.contains(&"c"));
        assert!(slugs.contains(&"d"));
    }

    #[test]
    fn hybrid_search_combines_fts_and_vector_results_without_exact_match() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "people/alice",
            "Alice",
            "AI founder",
            "Alice works on AI agents and research.",
            "people",
        );
        insert_page(
            &conn,
            "people/bob",
            "Bob",
            "Cloud founder",
            "Cloud infrastructure and data systems.",
            "people",
        );
        insert_page(
            &conn,
            "companies/acme",
            "Acme",
            "AI company",
            "AI agents platform for founders.",
            "companies",
        );
        embed::run(&conn, None, true, false).expect("embed pages");

        let results = hybrid_search("AI founder", None, &conn, 1000).expect("hybrid search");
        let slugs: Vec<_> = results.iter().map(|result| result.slug.as_str()).collect();

        assert!(slugs.contains(&"people/alice"));
        assert!(slugs.contains(&"companies/acme"));
    }

    #[test]
    fn hybrid_search_applies_wing_filter_to_both_subqueries() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "people/alice",
            "Alice",
            "AI founder",
            "Alice works on AI agents and research.",
            "people",
        );
        insert_page(
            &conn,
            "companies/acme",
            "Acme",
            "AI company",
            "AI agents platform for founders.",
            "companies",
        );
        embed::run(&conn, None, true, false).expect("embed pages");

        let results =
            hybrid_search("AI founder", Some("people"), &conn, 1000).expect("hybrid search");

        assert!(!results.is_empty());
        assert!(results.iter().all(|result| result.wing == "people"));
    }

    #[test]
    fn read_merge_strategy_defaults_to_set_union() {
        let conn = open_test_db();

        let strategy = read_merge_strategy(&conn).expect("merge strategy");

        assert_eq!(strategy, SearchMergeStrategy::SetUnion);
    }

    /// Regression: issue #37 — question marks in natural-language queries must
    /// not trigger FTS5 syntax errors in the hybrid search path.
    #[test]
    fn hybrid_search_accepts_question_mark_in_query() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/rust",
            "Rust",
            "Systems language",
            "Rust is a systems programming language focused on safety.",
            "concepts",
        );
        embed::run(&conn, None, true, false).expect("embed pages");

        let results =
            hybrid_search("what is rust?", None, &conn, 1000).expect("hybrid search with ?");
        assert!(!results.is_empty());
    }

    /// Regression: issue #37 — "AND?" must be safe on the natural-language path
    /// but still yield no results because no content-bearing terms survive.
    #[test]
    fn hybrid_search_returns_empty_for_operator_only_query() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/rust",
            "Rust",
            "Systems language",
            "Rust is a systems programming language focused on safety.",
            "concepts",
        );
        embed::run(&conn, None, true, false).expect("embed pages");

        let results = hybrid_search("AND?", None, &conn, 1000).expect("hybrid search with AND?");
        assert!(results.is_empty());
    }

    #[test]
    fn hybrid_search_returns_empty_for_punctuation_only_query() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/rust",
            "Rust",
            "Systems language",
            "Rust is a systems programming language focused on safety.",
            "concepts",
        );
        embed::run(&conn, None, true, false).expect("embed pages");

        let results = hybrid_search("???***", None, &conn, 1000)
            .expect("hybrid search with punctuation only");
        assert!(results.is_empty());
    }

    /// Regression: review blocker — commas, periods, apostrophes, slashes,
    /// semicolons, and `=` all trigger FTS5 syntax errors when passed raw.
    /// hybrid_search must sanitize all of them on the natural-language path.
    #[test]
    fn hybrid_search_accepts_comma_and_period_in_query() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/rust",
            "Rust",
            "Systems language",
            "Rust is a systems programming language focused on safety.",
            "concepts",
        );
        embed::run(&conn, None, true, false).expect("embed pages");

        assert!(hybrid_search("hello, world.", None, &conn, 1000).is_ok());
    }

    #[test]
    fn hybrid_search_accepts_apostrophe_in_query() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/rust",
            "Rust",
            "Systems language",
            "Rust is a systems programming language.",
            "concepts",
        );
        embed::run(&conn, None, true, false).expect("embed pages");

        assert!(hybrid_search("what's rust's type system?", None, &conn, 1000).is_ok());
    }

    #[test]
    fn hybrid_search_accepts_slash_and_equals_in_query() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/rust",
            "Rust",
            "Systems language",
            "Rust is a systems programming language.",
            "concepts",
        );
        embed::run(&conn, None, true, false).expect("embed pages");

        assert!(hybrid_search("path/to/thing key=value", None, &conn, 1000).is_ok());
    }

    #[test]
    fn hybrid_search_accepts_semicolon_in_query() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/rust",
            "Rust",
            "Systems language",
            "Rust is a systems programming language.",
            "concepts",
        );
        embed::run(&conn, None, true, false).expect("embed pages");

        assert!(hybrid_search("memory; safety", None, &conn, 1000).is_ok());
    }
}
