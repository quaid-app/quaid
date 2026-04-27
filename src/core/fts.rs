use rusqlite::Connection;

use super::types::{SearchError, SearchResult};

/// Sanitize a natural-language query string for safe use as an FTS5 `MATCH`
/// expression.  Returns a plain list of words that the FTS5 query parser will
/// accept without a syntax error.
///
/// **Strategy:** the FTS5 query parser (with the `porter unicode61` tokenizer)
/// rejects *any* character that is not alphanumeric, whitespace, or a
/// recognised query-syntax operator.  This includes the obvious operators
/// (`?`, `*`, `+`, `"`, `(`, `)`, …) but also everyday punctuation such as
/// commas, periods, apostrophes, slashes, semicolons, `=`, `@`, `#`, and
/// dozens of others.  Maintaining an explicit allowlist of "bad" characters is
/// fragile — every undiscovered character is a latent crash.
///
/// Instead, this function keeps *only* Unicode alphanumeric characters and
/// whitespace; everything else is replaced with a space.  The replacement
/// happens at the Unicode character level, so international content
/// (accented letters, CJK, etc.) passes through unmodified.
///
/// FTS5 boolean keywords (`AND`, `OR`, `NOT`, `NEAR`) are then quoted so they
/// are treated as literal search terms rather than query operators.  Only the
/// uppercase variants are operators; lowercase `and`/`or`/`not` are safe and
/// left unquoted.  This handles inputs like `AND?` → stripped `AND` → quoted
/// `"AND"`.
///
/// Natural-language callers should pass this output to [`search_fts_tiered`],
/// which keeps the initial implicit-AND pass and only widens to OR when the
/// AND pass returns no results. The explicit FTS5 query interface
/// ([`search_fts`]) is intentionally *not* touched — it preserves full FTS5
/// syntax for expert callers.
pub(crate) fn sanitize_fts_query(raw: &str) -> String {
    const FTS5_KEYWORDS: &[&str] = &["AND", "OR", "NOT", "NEAR"];

    // Replace every character that is not alphanumeric (Unicode) or whitespace
    // with a space.  This is intentionally broad: the FTS5 query parser
    // rejects all punctuation except its own recognised operators, so stripping
    // everything non-alphanumeric is the only way to guarantee safety without
    // maintaining a fragile per-character blocklist.
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect();

    // Collapse whitespace, then quote any bare FTS5 boolean keyword so it is
    // treated as a literal search term rather than a query operator.
    // FTS5 keywords are case-sensitive: only uppercase AND/OR/NOT/NEAR are
    // operators; lowercase variants are safe literals and must not be quoted.
    cleaned
        .split_whitespace()
        .map(|token| {
            if FTS5_KEYWORDS.contains(&token) {
                format!("\"{token}\"")
            } else {
                token.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// FTS5 full-text search over the `page_fts` virtual table.
///
/// Returns at most `limit` results ranked by BM25 score (most relevant first).
/// When `wing_filter` is provided, only pages in that wing are returned.
/// Returns an empty vec on no matches (not an error).
///
/// **Explicit FTS5 semantics are preserved.** Quoted phrases, boolean operators
/// (`AND`, `OR`, `NOT`), and prefix wildcards (`*`) all work as documented by
/// SQLite FTS5.  Invalid syntax is propagated as `Err` — this is intentional for
/// expert callers using `quaid search --raw`. `search_fts_tiered` uses this
/// function as its precision-first AND pass before widening to OR fallback.
///
/// Default callers are sanitized upstream:
/// - `src/commands/search.rs` applies `sanitize_fts_query` unless `--raw` is set.
/// - `src/mcp/server.rs` (`memory_search`) always sanitizes.
/// - `hybrid_search` in `src/core/search.rs` sanitizes before calling
///   `search_fts_tiered`.
pub fn search_fts(
    query: &str,
    wing_filter: Option<&str>,
    collection_filter: Option<i64>,
    conn: &Connection,
    limit: usize,
) -> Result<Vec<SearchResult>, SearchError> {
    search_fts_internal(query, wing_filter, collection_filter, conn, limit, false)
}

pub fn search_fts_canonical(
    query: &str,
    wing_filter: Option<&str>,
    collection_filter: Option<i64>,
    conn: &Connection,
    limit: usize,
) -> Result<Vec<SearchResult>, SearchError> {
    search_fts_internal(query, wing_filter, collection_filter, conn, limit, true)
}

/// Expands a sanitized multi-token query into an explicit FTS5 OR chain.
///
/// Single-token or empty inputs are returned unchanged.
pub fn expand_fts_query_or(sanitized: &str) -> String {
    let tokens: Vec<_> = sanitized.split_whitespace().collect();
    if tokens.len() <= 1 {
        sanitized.to_owned()
    } else {
        tokens.join(" OR ")
    }
}

/// Natural-language FTS5 search with tiered AND→OR fallback for compound-term
/// recall.
///
/// Tries an AND query first (highest precision). If AND returns no results and
/// the sanitized query has more than one token, retries with an explicit OR
/// chain so documents matching any individual term are surfaced.
///
/// Callers must pass a **sanitized** query from [`sanitize_fts_query`].
pub fn search_fts_tiered(
    sanitized_query: &str,
    wing_filter: Option<&str>,
    collection_filter: Option<i64>,
    conn: &Connection,
    limit: usize,
) -> Result<Vec<SearchResult>, SearchError> {
    search_fts_tiered_internal(
        sanitized_query,
        wing_filter,
        collection_filter,
        conn,
        limit,
        false,
    )
}

/// Canonical-slug variant of [`search_fts_tiered`].
/// Returns slugs in `<collection>::<slug>` format.
pub fn search_fts_canonical_tiered(
    sanitized_query: &str,
    wing_filter: Option<&str>,
    collection_filter: Option<i64>,
    conn: &Connection,
    limit: usize,
) -> Result<Vec<SearchResult>, SearchError> {
    search_fts_tiered_internal(
        sanitized_query,
        wing_filter,
        collection_filter,
        conn,
        limit,
        true,
    )
}

fn search_fts_tiered_internal(
    sanitized_query: &str,
    wing_filter: Option<&str>,
    collection_filter: Option<i64>,
    conn: &Connection,
    limit: usize,
    canonical_slug: bool,
) -> Result<Vec<SearchResult>, SearchError> {
    // AND pass — highest precision; use this if it returns any results.
    let and_results =
        search_fts_internal(sanitized_query, wing_filter, collection_filter, conn, limit, canonical_slug)?;
    if !and_results.is_empty() {
        return Ok(and_results);
    }

    let or_query = expand_fts_query_or(sanitized_query);
    if or_query == sanitized_query {
        return Ok(Vec::new());
    }

    search_fts_internal(&or_query, wing_filter, collection_filter, conn, limit, canonical_slug)
}

fn search_fts_internal(
    query: &str,
    wing_filter: Option<&str>,
    collection_filter: Option<i64>,
    conn: &Connection,
    limit: usize,
    canonical_slug: bool,
) -> Result<Vec<SearchResult>, SearchError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let slug_expr = if canonical_slug {
        "c.name || '::' || p.slug"
    } else {
        "p.slug"
    };
    let collection_join = if canonical_slug {
        " JOIN collections c ON c.id = p.collection_id"
    } else {
        ""
    };
    let mut sql = format!(
        "SELECT {slug_expr}, p.title, p.summary, -bm25(page_fts) AS score, p.wing \
         FROM page_fts \
         JOIN pages p ON p.id = page_fts.rowid{collection_join} \
         WHERE page_fts MATCH ?1",
    );

    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    params.push(Box::new(trimmed.to_owned()));

    if let Some(wing) = wing_filter {
        sql.push_str(" AND p.wing = ?2");
        params.push(Box::new(wing.to_owned()));
    }

    if let Some(collection_id) = collection_filter {
        sql.push_str(" AND p.collection_id = ?");
        sql.push_str(&(params.len() + 1).to_string());
        params.push(Box::new(collection_id));
    }

    // bm25() returns negative values; ascending order = most relevant first.
    sql.push_str(" ORDER BY bm25(page_fts) LIMIT ?");
    sql.push_str(&(params.len() + 1).to_string());
    params.push(Box::new(limit as i64));

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        Ok(SearchResult {
            slug: row.get(0)?,
            title: row.get(1)?,
            summary: row.get(2)?,
            score: row.get(3)?,
            wing: row.get(4)?,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;

    fn open_test_db() -> Connection {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        // Leak TempDir so the DB file stays alive for the test.
        std::mem::forget(dir);
        conn
    }

    fn insert_page(
        conn: &Connection,
        slug: &str,
        title: &str,
        wing: &str,
        summary: &str,
        compiled_truth: &str,
    ) {
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, \
                                timeline, frontmatter, wing, room, version) \
             VALUES (?1, 'concept', ?2, ?3, ?4, '', '{}', ?5, '', 1)",
            rusqlite::params![slug, title, summary, compiled_truth, wing],
        )
        .unwrap();
    }

    // ── search on empty DB ──────────────────────────────────────

    #[test]
    fn search_on_empty_db_returns_empty_vec() {
        let conn = open_test_db();
        let results = search_fts("anything", None, None, &conn, 1000).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_with_empty_query_returns_empty_vec() {
        let conn = open_test_db();
        insert_page(&conn, "test/a", "Test A", "test", "summary", "content");
        let results = search_fts("", None, None, &conn, 1000).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_with_whitespace_query_returns_empty_vec() {
        let conn = open_test_db();
        insert_page(&conn, "test/a", "Test A", "test", "summary", "content");
        let results = search_fts("   ", None, None, &conn, 1000).unwrap();
        assert!(results.is_empty());
    }

    // ── basic keyword match ─────────────────────────────────────

    #[test]
    fn search_finds_page_by_content_keyword() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/ml",
            "Machine Learning",
            "concepts",
            "ML overview",
            "Machine learning is a branch of artificial intelligence.",
        );

        let results = search_fts("machine learning", None, None, &conn, 1000).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].slug, "concepts/ml");
        assert_eq!(results[0].title, "Machine Learning");
        assert!(results[0].score > 0.0);
    }

    #[test]
    fn search_finds_page_by_title_keyword() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "people/alice",
            "Alice Johnson",
            "people",
            "Engineer at Acme",
            "Works on distributed systems.",
        );

        let results = search_fts("alice", None, None, &conn, 1000).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].slug, "people/alice");
    }

    #[test]
    fn search_returns_no_match_for_absent_term() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/ml",
            "Machine Learning",
            "concepts",
            "ML overview",
            "Machine learning is a branch of artificial intelligence.",
        );

        let results = search_fts("zzzznonexistent", None, None, &conn, 1000).unwrap();
        assert!(results.is_empty());
    }

    // ── wing filter ─────────────────────────────────────────────

    #[test]
    fn search_with_wing_filter_returns_only_matching_wing() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "people/alice",
            "Alice",
            "people",
            "Engineer",
            "Expert in fundraising and venture capital.",
        );
        insert_page(
            &conn,
            "companies/acme",
            "Acme Corp",
            "companies",
            "Startup",
            "A startup focused on fundraising technology.",
        );

        let results = search_fts("fundraising", Some("companies"), None, &conn, 1000).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].slug, "companies/acme");
    }

    #[test]
    fn search_without_wing_filter_returns_all_matching_pages() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "people/alice",
            "Alice",
            "people",
            "Engineer",
            "Expert in fundraising and venture capital.",
        );
        insert_page(
            &conn,
            "companies/acme",
            "Acme Corp",
            "companies",
            "Startup",
            "A startup focused on fundraising technology.",
        );

        let results = search_fts("fundraising", None, None, &conn, 1000).unwrap();
        assert_eq!(results.len(), 2);
    }

    // ── BM25 ranking ────────────────────────────────────────────

    #[test]
    fn search_results_are_ranked_by_relevance() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/ai-deep",
            "AI Deep Dive",
            "concepts",
            "Deep AI study",
            "Artificial intelligence is transforming everything. \
             Intelligence research continues. Artificial intelligence systems \
             are being deployed everywhere. Intelligence is key.",
        );
        insert_page(
            &conn,
            "concepts/ai-intro",
            "AI Introduction",
            "concepts",
            "Brief AI mention",
            "A brief note about intelligence in computing.",
        );

        let results = search_fts("intelligence", None, None, &conn, 1000).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].score >= results[1].score);
    }

    // ── result struct correctness ───────────────────────────────

    #[test]
    fn search_result_contains_correct_fields() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "people/bob",
            "Bob Smith",
            "people",
            "Bob is a researcher",
            "Bob works on quantum computing research.",
        );

        let results = search_fts("quantum", None, None, &conn, 1000).unwrap();
        assert_eq!(results.len(), 1);

        let r = &results[0];
        assert_eq!(r.slug, "people/bob");
        assert_eq!(r.title, "Bob Smith");
        assert_eq!(r.summary, "Bob is a researcher");
        assert_eq!(r.wing, "people");
        assert!(r.score > 0.0);
    }

    // ── sanitize_fts_query ──────────────────────────────────────

    #[test]
    fn sanitize_strips_question_mark() {
        assert_eq!(sanitize_fts_query("what is rust?"), "what is rust");
    }

    #[test]
    fn sanitize_strips_multiple_special_chars() {
        assert_eq!(
            sanitize_fts_query("(hello) + world: foo?"),
            "hello world foo"
        );
    }

    #[test]
    fn sanitize_collapses_whitespace() {
        assert_eq!(sanitize_fts_query("  hello   world  "), "hello world");
    }

    #[test]
    fn sanitize_returns_empty_for_only_special_chars() {
        assert_eq!(sanitize_fts_query("???***"), "");
    }

    #[test]
    fn sanitize_preserves_plain_words() {
        assert_eq!(sanitize_fts_query("machine learning"), "machine learning");
    }

    #[test]
    fn sanitize_quotes_bare_fts5_and_keyword() {
        // "AND?" → strip "?" → "AND" → quoted to prevent FTS5 operator error.
        assert_eq!(sanitize_fts_query("AND?"), "\"AND\"");
    }

    #[test]
    fn sanitize_quotes_bare_fts5_boolean_keywords() {
        assert_eq!(sanitize_fts_query("OR NOT NEAR"), "\"OR\" \"NOT\" \"NEAR\"");
    }

    #[test]
    fn sanitize_preserves_lowercase_and_or_not_as_plain_words() {
        // Lowercase and/or/not are not FTS5 operators — leave them unquoted.
        assert_eq!(sanitize_fts_query("cats and dogs"), "cats and dogs");
    }

    // ── regressions: punctuation beyond '?' (review blocker) ────────────────

    #[test]
    fn sanitize_strips_comma() {
        assert_eq!(sanitize_fts_query("hello, world"), "hello world");
    }

    #[test]
    fn sanitize_strips_period() {
        assert_eq!(sanitize_fts_query("hello. world"), "hello world");
    }

    #[test]
    fn sanitize_strips_apostrophe() {
        assert_eq!(sanitize_fts_query("what's up"), "what s up");
    }

    #[test]
    fn sanitize_strips_slash() {
        assert_eq!(sanitize_fts_query("path/to/thing"), "path to thing");
    }

    #[test]
    fn sanitize_strips_semicolon() {
        assert_eq!(sanitize_fts_query("key; value"), "key value");
    }

    #[test]
    fn sanitize_strips_equals() {
        assert_eq!(sanitize_fts_query("key=value"), "key value");
    }

    #[test]
    fn sanitize_strips_at_sign() {
        assert_eq!(sanitize_fts_query("user@host"), "user host");
    }

    #[test]
    fn sanitize_natural_language_sentence_with_mixed_punctuation() {
        // Full natural-language sentences must survive sanitization without crashing.
        let input = "What do I know about Alice's work on Quaid, specifically the FTS5 fix?";
        let result = sanitize_fts_query(input);
        assert!(!result.contains(','));
        assert!(!result.contains('\''));
        assert!(!result.contains('?'));
        assert!(result.contains("Alice"));
        assert!(result.contains("Quaid"));
    }

    // ── explicit FTS5 semantics (search_fts is the expert interface) ─────────

    #[test]
    fn search_fts_accepts_quoted_phrase() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/rust",
            "Rust Language",
            "concepts",
            "Systems programming",
            "Rust is a systems programming language.",
        );

        // Explicit FTS5 phrase query must pass through unmodified and match.
        let results = search_fts("\"systems programming\"", None, None, &conn, 1000).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].slug, "concepts/rust");
    }

    #[test]
    fn search_fts_accepts_boolean_and_operator() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/rust",
            "Rust Language",
            "concepts",
            "Systems programming",
            "Rust is a systems programming language.",
        );

        // FTS5 boolean AND operator must work for expert users.
        let results = search_fts("systems AND programming", None, None, &conn, 1000).unwrap();
        assert!(!results.is_empty());
    }

    /// Regression: issue #37 — search_fts() is the expert FTS5 interface.
    /// Invalid FTS5 syntax MUST propagate as Err (intended error contract).
    /// Natural-language callers must sanitize before calling search_fts.
    #[test]
    fn search_fts_returns_error_on_invalid_fts5_syntax() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "test/a",
            "Test",
            "test",
            "summary",
            "content about rust",
        );

        // A bare `?` is not valid FTS5 syntax — Err is the contract.
        let result = search_fts("rust?", None, None, &conn, 1000);
        assert!(
            result.is_err(),
            "search_fts must propagate FTS5 syntax errors"
        );
    }

    // ── expand_fts_query_or ──────────────────────────────────────

    #[test]
    fn expand_fts_query_or_joins_multi_token_query_with_or() {
        assert_eq!(
            expand_fts_query_or("neural network inference"),
            "neural OR network OR inference"
        );
    }

    #[test]
    fn expand_fts_query_or_leaves_single_token_query_unchanged() {
        assert_eq!(expand_fts_query_or("inference"), "inference");
    }

    #[test]
    fn expand_fts_query_or_leaves_empty_query_unchanged() {
        assert_eq!(expand_fts_query_or(""), "");
    }

    // ── search_fts_tiered: OR fallback for compound-term recall (issues #67, #69) ──

    /// Regression #67/#69: "neural network inference" returned zero results when
    /// documents contain the terms in different pages. `search_fts_tiered` must
    /// fall back to OR and surface those pages.
    #[test]
    fn search_tiered_compound_term_falls_back_to_or_when_and_empty() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/inference-engine",
            "Inference Engine",
            "concepts",
            "Model serving",
            "An inference engine serves trained embedding models in production.",
        );

        // AND search returns zero — no single page has all three tokens.
        let and_results = search_fts("neural network inference", None, None, &conn, 1000).unwrap();
        assert!(
            and_results.is_empty(),
            "AND search must return empty when no page contains all three tokens"
        );

        // Natural search must fall back to OR and find all three pages.
        let results =
            search_fts_tiered("neural network inference", None, None, &conn, 1000).unwrap();
        assert!(
            !results.is_empty(),
            "OR fallback must surface pages containing any of the query tokens"
        );
        assert_eq!(results[0].slug, "concepts/inference-engine");
    }

    /// Precision-first: when AND finds results, OR fallback must NOT be triggered.
    #[test]
    fn search_tiered_returns_and_results_without_or_fallback() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/combined",
            "Neural Network Inference",
            "concepts",
            "Combined page",
            "Neural network inference is the deployment of a trained model.",
        );
        insert_page(
            &conn,
            "concepts/neural",
            "Neural only",
            "concepts",
            "Neural",
            "A standalone neural science article.",
        );
        insert_page(
            &conn,
            "concepts/inference",
            "Inference only",
            "concepts",
            "Inference",
            "Inference in formal logic.",
        );

        let and_results = search_fts("neural network inference", None, None, &conn, 1000).unwrap();
        let tiered_results =
            search_fts_tiered("neural network inference", None, None, &conn, 1000).unwrap();

        assert_eq!(tiered_results.len(), and_results.len());
        assert_eq!(tiered_results[0].slug, "concepts/combined");
    }

    /// Single-token query: no OR fallback attempted; empty returns empty.
    #[test]
    fn search_tiered_single_token_empty_corpus_returns_empty() {
        let conn = open_test_db();
        let results = search_fts_tiered("zzznomatch", None, None, &conn, 1000).unwrap();
        assert!(
            results.is_empty(),
            "single-token miss must return empty, not trigger OR fallback"
        );
    }

    /// Empty sanitized query: no crash, empty vec returned.
    #[test]
    fn search_tiered_empty_query_returns_empty() {
        let conn = open_test_db();
        let results = search_fts_tiered("", None, None, &conn, 1000).unwrap();
        assert!(results.is_empty());
    }

    /// Wing filter is respected in OR-fallback path.
    #[test]
    fn search_tiered_or_fallback_respects_wing_filter() {
        let conn = open_test_db();
        // Same token appears in two wings; filter must restrict to one.
        insert_page(
            &conn,
            "people/alice",
            "Alice",
            "people",
            "Engineer",
            "Alice works on inference engines.",
        );
        insert_page(
            &conn,
            "companies/acme",
            "Acme",
            "companies",
            "Startup",
            "Acme builds inference products.",
        );

        // "inference cloud" AND would miss everything; OR fallback fires.
        let results = search_fts_tiered("inference cloud", Some("people"), None, &conn, 1000)
            .unwrap();
        for r in &results {
            assert_eq!(r.wing, "people", "OR fallback must respect wing filter");
        }
    }
}
