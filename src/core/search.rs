use std::collections::HashMap;

use rusqlite::Connection;

use super::collections::{self, OpKind, SlugResolution};
use super::fts::{
    sanitize_fts_query, search_fts_canonical_tiered_with_namespace_filtered,
    search_fts_tiered_with_namespace_filtered,
};
use super::inference::{
    search_vec_canonical_with_namespace_filtered, search_vec_with_namespace_filtered,
};
use super::types::{SearchError, SearchMergeStrategy, SearchResult};

/// Hybrid search with exact-slug short-circuit, FTS5, and vector search.
///
/// At most `limit` results are returned. The limit is pushed into the FTS5 query
/// and applied after the merge step to cap memory usage.
#[allow(dead_code)]
pub fn hybrid_search(
    query: &str,
    wing: Option<&str>,
    collection_filter: Option<i64>,
    include_superseded: bool,
    conn: &Connection,
    limit: usize,
) -> Result<Vec<SearchResult>, SearchError> {
    hybrid_search_with_namespace(
        query,
        wing,
        collection_filter,
        None,
        include_superseded,
        conn,
        limit,
    )
}

/// Namespace-aware variant of [`hybrid_search`].
#[allow(dead_code)]
pub fn hybrid_search_with_namespace(
    query: &str,
    wing: Option<&str>,
    collection_filter: Option<i64>,
    namespace_filter: Option<&str>,
    include_superseded: bool,
    conn: &Connection,
    limit: usize,
) -> Result<Vec<SearchResult>, SearchError> {
    hybrid_search_impl(
        query,
        wing,
        collection_filter,
        namespace_filter,
        include_superseded,
        conn,
        limit,
        false,
    )
}

/// Hybrid search returning canonical `<collection>::<slug>` identifiers.
#[allow(dead_code)]
pub fn hybrid_search_canonical(
    query: &str,
    wing: Option<&str>,
    collection_filter: Option<i64>,
    include_superseded: bool,
    conn: &Connection,
    limit: usize,
) -> Result<Vec<SearchResult>, SearchError> {
    hybrid_search_canonical_with_namespace(
        query,
        wing,
        collection_filter,
        None,
        include_superseded,
        conn,
        limit,
    )
}

/// Namespace-aware canonical-slug variant of [`hybrid_search`].
pub fn hybrid_search_canonical_with_namespace(
    query: &str,
    wing: Option<&str>,
    collection_filter: Option<i64>,
    namespace_filter: Option<&str>,
    include_superseded: bool,
    conn: &Connection,
    limit: usize,
) -> Result<Vec<SearchResult>, SearchError> {
    hybrid_search_impl(
        query,
        wing,
        collection_filter,
        namespace_filter,
        include_superseded,
        conn,
        limit,
        true,
    )
}

fn hybrid_search_impl(
    query: &str,
    wing: Option<&str>,
    collection_filter: Option<i64>,
    namespace_filter: Option<&str>,
    include_superseded: bool,
    conn: &Connection,
    limit: usize,
    canonical_slug: bool,
) -> Result<Vec<SearchResult>, SearchError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    if let Some(slug) = exact_slug_query(trimmed) {
        if let Some(result) = exact_slug_result_with_namespace(
            slug,
            wing,
            collection_filter,
            namespace_filter,
            include_superseded,
            conn,
            canonical_slug,
        )? {
            return Ok(vec![result]);
        }
    }

    let fts_safe = sanitize_fts_query(trimmed);
    if !has_natural_language_terms(&fts_safe) {
        return Ok(Vec::new());
    }

    let fts_results = if canonical_slug {
        search_fts_canonical_tiered_with_namespace_filtered(
            &fts_safe,
            wing,
            collection_filter,
            namespace_filter,
            include_superseded,
            conn,
            limit,
        )?
    } else {
        search_fts_tiered_with_namespace_filtered(
            &fts_safe,
            wing,
            collection_filter,
            namespace_filter,
            include_superseded,
            conn,
            limit,
        )?
    };
    let vec_results = if canonical_slug {
        search_vec_canonical_with_namespace_filtered(
            trimmed,
            10,
            wing,
            collection_filter,
            namespace_filter,
            include_superseded,
            conn,
        )?
    } else {
        search_vec_with_namespace_filtered(
            trimmed,
            10,
            wing,
            collection_filter,
            namespace_filter,
            include_superseded,
            conn,
        )?
    };

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

#[cfg(test)]
fn exact_slug_result(
    slug: &str,
    wing: Option<&str>,
    collection_filter: Option<i64>,
    include_superseded: bool,
    conn: &Connection,
    canonical_slug: bool,
) -> Result<Option<SearchResult>, SearchError> {
    exact_slug_result_with_namespace(
        slug,
        wing,
        collection_filter,
        None,
        include_superseded,
        conn,
        canonical_slug,
    )
}

fn exact_slug_result_with_namespace(
    slug: &str,
    wing: Option<&str>,
    collection_filter: Option<i64>,
    namespace_filter: Option<&str>,
    include_superseded: bool,
    conn: &Connection,
    canonical_slug: bool,
) -> Result<Option<SearchResult>, SearchError> {
    if canonical_slug {
        return exact_slug_result_canonical_with_namespace(
            slug,
            wing,
            collection_filter,
            namespace_filter,
            include_superseded,
            conn,
        );
    }

    let mut query = String::from("SELECT slug, title, summary, wing FROM pages WHERE slug = ?1");
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(slug.to_owned())];

    if let Some(wing) = wing {
        query.push_str(" AND wing = ?");
        query.push_str(&(params.len() + 1).to_string());
        params.push(Box::new(wing.to_owned()));
    }
    if let Some(collection_id) = collection_filter {
        query.push_str(" AND collection_id = ?");
        query.push_str(&(params.len() + 1).to_string());
        params.push(Box::new(collection_id));
    }
    if let Some(namespace) = namespace_filter {
        if namespace.is_empty() {
            query.push_str(" AND namespace = ?");
            query.push_str(&(params.len() + 1).to_string());
            params.push(Box::new(String::new()));
        } else {
            query.push_str(" AND (namespace = ?");
            query.push_str(&(params.len() + 1).to_string());
            query.push_str(" OR namespace = '')");
            params.push(Box::new(namespace.to_owned()));
        }
    }
    if !include_superseded {
        query.push_str(" AND superseded_by IS NULL");
    }
    if let Some(namespace) = namespace_filter.filter(|namespace| !namespace.is_empty()) {
        query.push_str(" ORDER BY CASE WHEN namespace = ?");
        query.push_str(&(params.len() + 1).to_string());
        query.push_str(" THEN 0 ELSE 1 END");
        params.push(Box::new(namespace.to_owned()));
    }
    query.push_str(" LIMIT 1");

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let result = conn.query_row(&query, param_refs.as_slice(), |row| {
        Ok(SearchResult {
            slug: row.get(0)?,
            title: row.get(1)?,
            summary: row.get(2)?,
            score: 1.0,
            wing: row.get(3)?,
        })
    });

    match result {
        Ok(result) => Ok(Some(result)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) => Err(SearchError::from(err)),
    }
}

#[cfg(test)]
fn exact_slug_result_canonical(
    slug: &str,
    wing: Option<&str>,
    collection_filter: Option<i64>,
    include_superseded: bool,
    conn: &Connection,
) -> Result<Option<SearchResult>, SearchError> {
    exact_slug_result_canonical_with_namespace(
        slug,
        wing,
        collection_filter,
        None,
        include_superseded,
        conn,
    )
}

fn exact_slug_result_canonical_with_namespace(
    slug: &str,
    wing: Option<&str>,
    collection_filter: Option<i64>,
    namespace_filter: Option<&str>,
    include_superseded: bool,
    conn: &Connection,
) -> Result<Option<SearchResult>, SearchError> {
    if let Some(collection_id) = collection_filter {
        return exact_slug_result_canonical_for_collection_with_namespace(
            slug,
            wing,
            collection_id,
            namespace_filter,
            include_superseded,
            conn,
        );
    }

    let resolved = match collections::parse_slug(conn, slug, OpKind::Read) {
        Ok(SlugResolution::Resolved {
            collection_id,
            collection_name,
            slug,
        }) => (collection_id, collection_name, slug),
        Ok(SlugResolution::NotFound { .. }) => {
            return Ok(None);
        }
        Ok(SlugResolution::Ambiguous { slug, candidates }) => {
            return Err(SearchError::Ambiguous {
                slug,
                candidates: candidates
                    .into_iter()
                    .map(|candidate| candidate.full_address)
                    .collect::<Vec<_>>()
                    .join(", "),
            });
        }
        Err(collections::CollectionError::Sqlite(err)) => return Err(SearchError::from(err)),
        Err(_) => return Ok(None),
    };

    let result = query_exact_slug_canonical(
        &resolved.2,
        wing,
        resolved.0,
        namespace_filter,
        include_superseded,
        conn,
    );

    match result {
        Ok(result) => Ok(Some(result)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) => Err(SearchError::from(err)),
    }
}

#[cfg(test)]
fn exact_slug_result_canonical_for_collection(
    slug: &str,
    wing: Option<&str>,
    collection_id: i64,
    include_superseded: bool,
    conn: &Connection,
) -> Result<Option<SearchResult>, SearchError> {
    exact_slug_result_canonical_for_collection_with_namespace(
        slug,
        wing,
        collection_id,
        None,
        include_superseded,
        conn,
    )
}

fn exact_slug_result_canonical_for_collection_with_namespace(
    slug: &str,
    wing: Option<&str>,
    collection_id: i64,
    namespace_filter: Option<&str>,
    include_superseded: bool,
    conn: &Connection,
) -> Result<Option<SearchResult>, SearchError> {
    let stripped_slug = if let Some((collection_name, relative_slug)) = slug.split_once("::") {
        match collections::get_by_name(conn, collection_name) {
            Ok(Some(collection)) if collection.id == collection_id => relative_slug.to_owned(),
            Ok(Some(_)) | Ok(None) => return Ok(None),
            Err(collections::CollectionError::Sqlite(err)) => return Err(SearchError::from(err)),
            Err(_) => return Ok(None),
        }
    } else {
        slug.to_owned()
    };

    let result = query_exact_slug_canonical(
        &stripped_slug,
        wing,
        collection_id,
        namespace_filter,
        include_superseded,
        conn,
    );

    match result {
        Ok(result) => Ok(Some(result)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) => Err(SearchError::from(err)),
    }
}

fn query_exact_slug_canonical(
    slug: &str,
    wing: Option<&str>,
    collection_id: i64,
    namespace_filter: Option<&str>,
    include_superseded: bool,
    conn: &Connection,
) -> Result<SearchResult, rusqlite::Error> {
    let mut query = String::from(
        "SELECT c.name || '::' || p.slug, p.title, p.summary, p.wing
         FROM pages p
         JOIN collections c ON c.id = p.collection_id
         WHERE p.collection_id = ?1 AND p.slug = ?2",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(collection_id), Box::new(slug.to_owned())];

    if let Some(wing) = wing {
        query.push_str(" AND p.wing = ?");
        query.push_str(&(params.len() + 1).to_string());
        params.push(Box::new(wing.to_owned()));
    }
    if let Some(namespace) = namespace_filter {
        if namespace.is_empty() {
            query.push_str(" AND p.namespace = ?");
            query.push_str(&(params.len() + 1).to_string());
            params.push(Box::new(String::new()));
        } else {
            query.push_str(" AND (p.namespace = ?");
            query.push_str(&(params.len() + 1).to_string());
            query.push_str(" OR p.namespace = '')");
            params.push(Box::new(namespace.to_owned()));
        }
    }
    if !include_superseded {
        query.push_str(" AND p.superseded_by IS NULL");
    }
    if let Some(namespace) = namespace_filter.filter(|namespace| !namespace.is_empty()) {
        query.push_str(" ORDER BY CASE WHEN p.namespace = ?");
        query.push_str(&(params.len() + 1).to_string());
        query.push_str(" THEN 0 ELSE 1 END");
        params.push(Box::new(namespace.to_owned()));
    }
    query.push_str(" LIMIT 1");

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    conn.query_row(&query, param_refs.as_slice(), |row| {
        Ok(SearchResult {
            slug: row.get(0)?,
            title: row.get(1)?,
            summary: row.get(2)?,
            score: 1.0,
            wing: row.get(3)?,
        })
    })
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
    use std::collections::BTreeSet;

    use rusqlite::{params, Connection};

    use super::*;
    use crate::commands::embed;
    use crate::core::db;
    use crate::core::fts::{sanitize_fts_query, search_fts};
    use crate::core::inference::{search_vec, search_vec_canonical};

    fn open_test_db() -> Connection {
        let dir = tempfile::TempDir::new().expect("create temp dir");
        let db_path = dir.path().join("test_memory.db");
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

    fn insert_collection(conn: &Connection, name: &str) -> i64 {
        conn.execute(
            "INSERT INTO collections (name, root_path, state, writable, is_write_target)
             VALUES (?1, ?2, 'active', 1, 0)",
            params![name, format!("/{name}")],
        )
        .expect("insert collection");
        conn.last_insert_rowid()
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

    fn insert_page_in_collection(
        conn: &Connection,
        collection_id: i64,
        slug: &str,
        title: &str,
        summary: &str,
        truth: &str,
        wing: &str,
    ) {
        let mut hex = String::new();
        for byte in format!("{collection_id}-{slug}").as_bytes() {
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
            "INSERT INTO pages (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version) \
             VALUES (?1, ?2, ?3, 'person', ?4, ?5, ?6, '', '{}', ?7, '', 1)",
            rusqlite::params![collection_id, slug, uuid, title, summary, truth, wing],
        )
        .expect("insert page");
    }

    fn supersede_page(conn: &Connection, predecessor_slug: &str, successor_slug: &str) {
        let successor_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE slug = ?1",
                [successor_slug],
                |row| row.get(0),
            )
            .expect("successor id");
        conn.execute(
            "UPDATE pages SET superseded_by = ?1 WHERE slug = ?2",
            rusqlite::params![successor_id, predecessor_slug],
        )
        .expect("mark predecessor superseded");
    }

    #[test]
    fn hybrid_search_returns_empty_for_blank_query() {
        let conn = open_test_db();

        let results = hybrid_search("   ", None, None, false, &conn, 1000).expect("hybrid search");

        assert!(results.is_empty());
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

        let results =
            hybrid_search("people/alice", None, None, false, &conn, 1000).expect("hybrid search");

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

        let results = hybrid_search("[[people/alice]]", None, None, false, &conn, 1000)
            .expect("hybrid search");

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

        let results =
            hybrid_search("AI founder", None, None, false, &conn, 1000).expect("hybrid search");
        let slugs: Vec<_> = results.iter().map(|result| result.slug.as_str()).collect();

        assert!(slugs.contains(&"people/alice"));
        assert!(slugs.contains(&"companies/acme"));
    }

    #[test]
    fn hybrid_search_uses_tiered_fts_recall_when_and_and_vector_are_empty() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/neural",
            "Neural Networks",
            "Representation learning",
            "Neural networks learn useful representations.",
            "concepts",
        );
        insert_page(
            &conn,
            "concepts/inference-engine",
            "Inference Engine",
            "Model serving",
            "An inference engine deploys trained models.",
            "concepts",
        );

        let sanitized = sanitize_fts_query("neural network inference");
        assert!(
            search_fts(&sanitized, None, None, &conn, 1000)
                .expect("AND-only FTS query")
                .is_empty(),
            "the deterministic proof corpus must force the implicit-AND FTS pass to miss"
        );
        assert!(
            search_vec("neural network inference", 10, None, None, &conn)
                .expect("vector search without embeddings")
                .is_empty(),
            "no embeddings are written in this proof, so vector recall must stay empty"
        );

        let results = hybrid_search("neural network inference", None, None, false, &conn, 1000)
            .expect("hybrid search");

        assert_eq!(
            results
                .iter()
                .map(|result| result.slug.clone())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "concepts/inference-engine".to_owned(),
                "concepts/neural".to_owned(),
            ]),
            "hybrid_search should recover compound-term recall from the tiered FTS arm alone"
        );
    }

    #[test]
    fn hybrid_search_canonical_preserves_collection_prefix_on_tiered_fts_recall() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/neural",
            "Neural Networks",
            "Representation learning",
            "Neural networks learn useful representations.",
            "concepts",
        );
        insert_page(
            &conn,
            "concepts/inference-engine",
            "Inference Engine",
            "Model serving",
            "An inference engine deploys trained models.",
            "concepts",
        );

        assert!(
            search_vec_canonical("neural network inference", 10, None, None, &conn)
                .expect("canonical vector search without embeddings")
                .is_empty(),
            "canonical hybrid proof must not depend on vector quality"
        );

        let results =
            hybrid_search_canonical("neural network inference", None, None, false, &conn, 1000)
                .expect("canonical hybrid search");

        assert_eq!(
            results
                .iter()
                .map(|result| result.slug.clone())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "default::concepts/inference-engine".to_owned(),
                "default::concepts/neural".to_owned(),
            ]),
            "canonical hybrid results should keep the collection prefix on FTS fallback hits"
        );
    }

    #[test]
    fn exact_slug_result_canonical_hides_superseded_pages_by_default() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "facts/a",
            "Fact A",
            "Archived",
            "Historical version",
            "facts",
        );
        insert_page(
            &conn,
            "facts/b",
            "Fact B",
            "Current",
            "Current version",
            "facts",
        );
        supersede_page(&conn, "facts/a", "facts/b");

        let default_result =
            exact_slug_result_canonical("facts/a", None, None, false, &conn).expect("head-only");
        let historical_result =
            exact_slug_result_canonical("facts/a", None, None, true, &conn).expect("history");

        assert!(default_result.is_none());
        assert_eq!(
            historical_result.expect("historical result").slug,
            "default::facts/a"
        );
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

        let results = hybrid_search("AI founder", Some("people"), None, false, &conn, 1000)
            .expect("hybrid search");

        assert!(!results.is_empty());
        assert!(results.iter().all(|result| result.wing == "people"));
    }

    #[test]
    fn hybrid_search_namespace_filter_includes_global_and_excludes_other_namespaces() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "notes/global",
            "Global",
            "Global",
            "sharedtoken",
            "notes",
        );
        conn.execute(
            "INSERT INTO pages
                 (namespace, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
             VALUES ('test-ns', 'notes/private', ?1, 'concept', 'Private', 'Private', 'privatetoken', '', '{}', 'notes', '', 1)",
            [uuid::Uuid::now_v7().to_string()],
        )
        .expect("insert namespaced page");

        let namespaced = hybrid_search_canonical_with_namespace(
            "sharedtoken privatetoken",
            None,
            None,
            Some("test-ns"),
            false,
            &conn,
            10,
        )
        .expect("namespaced query");
        let global_only = hybrid_search_canonical_with_namespace(
            "privatetoken",
            None,
            None,
            Some(""),
            false,
            &conn,
            10,
        )
        .expect("global query");

        assert!(namespaced
            .iter()
            .any(|result| result.slug == "default::notes/global"));
        assert!(namespaced
            .iter()
            .any(|result| result.slug == "default::notes/private"));
        assert!(global_only.is_empty());
    }

    #[test]
    fn read_merge_strategy_defaults_to_set_union() {
        let conn = open_test_db();

        let strategy = read_merge_strategy(&conn).expect("merge strategy");

        assert_eq!(strategy, SearchMergeStrategy::SetUnion);
    }

    #[test]
    fn read_merge_strategy_honors_rrf_config() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO config (key, value) VALUES ('search_merge_strategy', 'rrf')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )
        .expect("insert config");

        let strategy = read_merge_strategy(&conn).expect("merge strategy");

        assert_eq!(strategy, SearchMergeStrategy::Rrf);
    }

    #[test]
    fn read_merge_strategy_propagates_sql_errors() {
        let conn = open_test_db();
        conn.execute("DROP TABLE config", []).expect("drop config");

        let err = read_merge_strategy(&conn).expect_err("missing config table should fail");

        assert!(matches!(err, SearchError::Sqlite(_)));
    }

    #[test]
    fn hybrid_search_uses_rrf_when_configured() {
        let conn = open_test_db();
        insert_page(
            &conn,
            "concepts/rust",
            "Rust",
            "Systems language",
            "Rust is a systems programming language focused on memory safety.",
            "concepts",
        );
        conn.execute(
            "INSERT INTO config (key, value) VALUES ('search_merge_strategy', 'rrf')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )
        .expect("insert config");

        let results =
            hybrid_search("systems language", None, None, false, &conn, 1).expect("rrf search");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].slug, "concepts/rust");
    }

    #[test]
    fn exact_slug_result_applies_wing_and_collection_filters() {
        let conn = open_test_db();
        let default_collection_id: i64 = conn
            .query_row(
                "SELECT id FROM collections WHERE name = 'default'",
                [],
                |row| row.get(0),
            )
            .expect("default collection id");
        insert_page(
            &conn,
            "people/alice",
            "Alice",
            "Founder",
            "Alice works on AI agents.",
            "people",
        );

        let matching = exact_slug_result(
            "people/alice",
            Some("people"),
            Some(default_collection_id),
            false,
            &conn,
            false,
        )
        .expect("exact slug");
        let wrong_wing =
            exact_slug_result("people/alice", Some("companies"), None, false, &conn, false)
                .expect("wrong wing");
        let wrong_collection = exact_slug_result(
            "people/alice",
            None,
            Some(default_collection_id + 999),
            false,
            &conn,
            false,
        )
        .expect("wrong collection");

        assert_eq!(matching.expect("matching result").slug, "people/alice");
        assert!(wrong_wing.is_none());
        assert!(wrong_collection.is_none());
    }

    #[test]
    fn exact_slug_result_propagates_sql_errors() {
        let conn = open_test_db();
        conn.execute("DROP TABLE pages", []).expect("drop pages");

        let err = exact_slug_result("people/alice", None, None, false, &conn, false)
            .expect_err("missing pages table should fail");

        assert!(matches!(err, SearchError::Sqlite(_)));
    }

    #[test]
    fn hybrid_search_canonical_returns_ambiguous_error_for_exact_slug_query() {
        let conn = open_test_db();
        let work_collection_id = insert_collection(&conn, "work");
        insert_page(
            &conn,
            "people/alice",
            "Alice Default",
            "Founder",
            "Alice works on default agents.",
            "people",
        );
        insert_page_in_collection(
            &conn,
            work_collection_id,
            "people/alice",
            "Alice Work",
            "Operator",
            "Alice works on the work collection agents.",
            "people",
        );

        let err = hybrid_search_canonical("people/alice", None, None, false, &conn, 10)
            .expect_err("ambiguous canonical search should fail");

        assert!(matches!(
            err,
            SearchError::Ambiguous { slug, candidates }
                if slug == "people/alice"
                    && candidates.contains("default::people/alice")
                    && candidates.contains("work::people/alice")
        ));
    }

    #[test]
    fn exact_slug_result_canonical_returns_prefixed_slug_for_explicit_collection_address() {
        let conn = open_test_db();
        let work_collection_id = insert_collection(&conn, "work");
        insert_page_in_collection(
            &conn,
            work_collection_id,
            "people/alice",
            "Alice Work",
            "Operator",
            "Alice works on the work collection agents.",
            "people",
        );

        let result =
            exact_slug_result_canonical("work::people/alice", Some("people"), None, false, &conn)
                .expect("canonical exact slug")
                .expect("matching page");

        assert_eq!(result.slug, "work::people/alice");
        assert_eq!(result.wing, "people");
    }

    #[test]
    fn exact_slug_result_canonical_returns_none_when_wing_filter_excludes_match() {
        let conn = open_test_db();
        let work_collection_id = insert_collection(&conn, "work");
        insert_page_in_collection(
            &conn,
            work_collection_id,
            "people/alice",
            "Alice Work",
            "Operator",
            "Alice works on the work collection agents.",
            "people",
        );

        let result = exact_slug_result_canonical(
            "work::people/alice",
            Some("companies"),
            None,
            false,
            &conn,
        )
        .expect("canonical exact slug");

        assert!(result.is_none());
    }

    #[test]
    fn exact_slug_result_canonical_for_collection_propagates_collection_lookup_sql_errors() {
        let conn = open_test_db();
        conn.execute("DROP TABLE collections", [])
            .expect("drop collections");

        let err =
            exact_slug_result_canonical_for_collection("work::people/alice", None, 1, false, &conn)
                .expect_err("missing collections table should fail");

        assert!(matches!(err, SearchError::Sqlite(_)));
    }

    #[test]
    fn exact_slug_result_canonical_returns_none_for_invalid_bare_slug() {
        let conn = open_test_db();

        let result = exact_slug_result_canonical("/etc/passwd", None, None, false, &conn)
            .expect("invalid slug");

        assert!(result.is_none());
    }

    #[test]
    fn exact_slug_result_canonical_for_collection_handles_prefix_and_wing_filters() {
        let conn = open_test_db();
        let work_collection_id = insert_collection(&conn, "work");
        insert_page_in_collection(
            &conn,
            work_collection_id,
            "people/alice",
            "Alice Work",
            "Operator",
            "Alice works on the work collection agents.",
            "people",
        );

        let matching = exact_slug_result_canonical_for_collection(
            "work::people/alice",
            Some("people"),
            work_collection_id,
            false,
            &conn,
        )
        .expect("matching canonical exact slug");
        let wrong_prefix = exact_slug_result_canonical_for_collection(
            "default::people/alice",
            None,
            work_collection_id,
            false,
            &conn,
        )
        .expect("wrong prefix");
        let missing_prefix = exact_slug_result_canonical_for_collection(
            "missing::people/alice",
            None,
            work_collection_id,
            false,
            &conn,
        )
        .expect("missing prefix");
        let wrong_wing = exact_slug_result_canonical_for_collection(
            "people/alice",
            Some("companies"),
            work_collection_id,
            false,
            &conn,
        )
        .expect("wrong wing");

        assert_eq!(
            matching.expect("matching result").slug,
            "work::people/alice"
        );
        assert!(wrong_prefix.is_none());
        assert!(missing_prefix.is_none());
        assert!(wrong_wing.is_none());
    }

    #[test]
    fn merge_rrf_combines_ranked_results() {
        let fts = vec![result("a", 10.0), result("b", 9.0)];
        let vec = vec![result("b", 8.0), result("c", 7.0)];

        let results = merge_rrf(&fts, &vec);
        let slugs: Vec<_> = results.iter().map(|result| result.slug.as_str()).collect();

        assert_eq!(slugs, vec!["b", "a", "c"]);
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn normalize_score_returns_zero_when_max_is_zero() {
        assert_eq!(normalize_score(5.0, 0.0), 0.0);
        assert_eq!(max_score(&[]), 0.0);
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

        let results = hybrid_search("what is rust?", None, None, false, &conn, 1000)
            .expect("hybrid search with ?");
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

        let results =
            hybrid_search("AND?", None, None, false, &conn, 1000).expect("hybrid search with AND?");
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

        let results = hybrid_search("???***", None, None, false, &conn, 1000)
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

        assert!(hybrid_search("hello, world.", None, None, false, &conn, 1000).is_ok());
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

        assert!(
            hybrid_search("what's rust's type system?", None, None, false, &conn, 1000).is_ok()
        );
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

        assert!(hybrid_search("path/to/thing key=value", None, None, false, &conn, 1000).is_ok());
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

        assert!(hybrid_search("memory; safety", None, None, false, &conn, 1000).is_ok());
    }
}
