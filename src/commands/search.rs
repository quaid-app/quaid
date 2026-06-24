#![expect(
    clippy::print_stdout,
    reason = "CLI command prints user-facing output to stdout by design"
)]

use anyhow::Result;
use rusqlite::Connection;

use crate::core::fts::{sanitize_fts_query, search_fts, search_fts_tiered, FtsQuery};

#[expect(
    clippy::too_many_arguments,
    reason = "search command CLI accepts the documented user-facing flags directly; collapsing into a struct would obscure the dispatch boundary"
)]
pub fn run(
    db: &Connection,
    query: &str,
    wing: Option<String>,
    namespace: Option<&str>,
    limit: u32,
    include_superseded: bool,
    json: bool,
    raw: bool,
    hops: Option<u32>,
    relevance_floor: Option<f64>,
    max_chunks_per_doc: Option<usize>,
    mmr_lambda: Option<f64>,
) -> Result<()> {
    crate::core::namespace::validate_optional_namespace(namespace)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let namespace = namespace.or(Some(""));
    let effective_query = if raw {
        query.to_owned()
    } else {
        sanitize_fts_query(query)
    };
    let fts_query = FtsQuery {
        query: &effective_query,
        wing: wing.as_deref(),
        namespace,
        include_superseded,
        canonical: true,
        limit: limit as usize,
        ..Default::default()
    };
    let results = if raw {
        search_fts(db, fts_query)
    } else {
        search_fts_tiered(db, fts_query)
    };

    let results = match results {
        Ok(r) => r,
        Err(e) => {
            if json {
                println!("{}", serde_json::json!({"error": e.to_string()}));
            } else {
                return Err(e.into());
            }
            return Ok(());
        }
    };

    // Post-retrieval quality passes (dedup → floor → MMR); identity no-ops at
    // the seeded config defaults. The flag values, when given, override the
    // `search.max_chunks_per_doc_default` / `search.relevance_floor` /
    // `search.mmr_lambda` keys.
    let results = match apply_quality_passes(
        db,
        results,
        relevance_floor,
        max_chunks_per_doc,
        mmr_lambda,
        limit as usize,
    ) {
        Ok(filtered) => filtered,
        Err(e) => {
            if json {
                println!("{}", serde_json::json!({"error": e.to_string()}));
                return Ok(());
            }
            return Err(e.into());
        }
    };

    let results: Vec<_> = results.into_iter().take(limit as usize).collect();

    // Apply graph expansion when the effective depth is non-zero. The CLI
    // `--hops` value, when given, overrides `config.graph_depth` for this
    // invocation; otherwise the seeded default (`1`) is used.
    let results = match expand_with_hops(db, results, hops) {
        Ok(expanded) => expanded,
        Err(e) => {
            if json {
                println!("{}", serde_json::json!({"error": e.to_string()}));
                return Ok(());
            }
            return Err(e.into());
        }
    };

    let results: Vec<_> = results.into_iter().take(limit as usize).collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else if results.is_empty() {
        println!("No results found.");
    } else {
        for r in &results {
            println!("{}: {}", r.slug, r.summary);
        }
    }

    Ok(())
}

fn apply_quality_passes(
    db: &Connection,
    results: Vec<crate::core::types::SearchResult>,
    relevance_floor: Option<f64>,
    max_chunks_per_doc: Option<usize>,
    mmr_lambda: Option<f64>,
    limit: usize,
) -> std::result::Result<Vec<crate::core::types::SearchResult>, crate::core::types::SearchError> {
    use crate::core::search::{
        apply_mmr, configured_max_chunks_per_doc, configured_mmr_lambda, configured_relevance_floor,
        dedup_chunks_per_page, filter_below_floor,
    };

    let max_per_page = match max_chunks_per_doc {
        Some(value) => value,
        None => configured_max_chunks_per_doc(db)?,
    };
    let floor = match relevance_floor {
        Some(value) => value.clamp(0.0, 1.0),
        None => configured_relevance_floor(db)?,
    };
    let lambda = match mmr_lambda {
        Some(value) => value.clamp(0.0, 1.0),
        None => configured_mmr_lambda(db)?,
    };
    let filtered = filter_below_floor(dedup_chunks_per_page(results, max_per_page), floor);
    Ok(apply_mmr(db, filtered, lambda, limit))
}

fn expand_with_hops(
    db: &Connection,
    mut results: Vec<crate::core::types::SearchResult>,
    hops: Option<u32>,
) -> std::result::Result<Vec<crate::core::types::SearchResult>, crate::core::types::SearchError> {
    use crate::core::search::{expand_graph, GraphExpansionConfig};

    if results.is_empty() {
        return Ok(results);
    }
    let cfg = GraphExpansionConfig::from_config(db)?;
    let depth = hops.unwrap_or(cfg.depth);
    if depth == 0 {
        return Ok(results);
    }
    let added = expand_graph(db, &results, depth, cfg.max_added, cfg.distance_decay, None)?;
    if !added.is_empty() {
        results.extend(added);
        results.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.slug.cmp(&b.slug))
        });
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;

    fn open_test_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("search_cmd_test.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        (dir, conn)
    }

    // D.1 — natural-language query with '?' does not error when sanitized
    #[test]
    fn run_sanitized_question_mark_query_returns_ok() {
        let (_dir, conn) = open_test_db();
        let result = run(
            &conn,
            "what is CLARITY?",
            None,
            None,
            10,
            false,
            false,
            false,
            None,
            None,
            None,
            None,
        );
        assert!(
            result.is_ok(),
            "sanitized '?' query must not error: {result:?}"
        );
    }

    // D.2 — natural-language query with apostrophe does not error
    #[test]
    fn run_sanitized_apostrophe_query_returns_ok() {
        let (_dir, conn) = open_test_db();
        let result = run(
            &conn,
            "it's a stablecoin",
            None,
            None,
            10,
            false,
            false,
            false,
            None,
            None,
            None,
            None,
        );
        assert!(
            result.is_ok(),
            "sanitized apostrophe query must not error: {result:?}"
        );
    }

    // D.3 — natural-language query with hyphens and dots does not error
    #[test]
    fn run_sanitized_hyphen_dot_query_returns_ok() {
        let (_dir, conn) = open_test_db();
        let result = run(
            &conn,
            "gpt-5.4 codex model",
            None,
            None,
            10,
            false,
            false,
            false,
            None,
            None,
            None,
            None,
        );
        assert!(
            result.is_ok(),
            "sanitized hyphen/dot query must not error: {result:?}"
        );
    }

    // D.4 — --json with sanitized query always produces valid JSON (exits Ok)
    #[test]
    fn run_json_mode_with_percent_query_returns_ok() {
        let (_dir, conn) = open_test_db();
        // '50% fee reduction' contains '%' — sanitized to '50 fee reduction'
        let result = run(
            &conn,
            "50% fee reduction",
            None,
            None,
            10,
            false,
            true,
            false,
            None,
            None,
            None,
            None,
        );
        assert!(
            result.is_ok(),
            "--json sanitized query must return Ok: {result:?}"
        );
    }

    // D.5 — --raw --json with invalid FTS5 syntax returns Ok (error JSON written to stdout)
    #[test]
    fn run_raw_json_mode_with_invalid_fts5_returns_ok_not_panic() {
        let (_dir, conn) = open_test_db();
        // '?invalid' is invalid FTS5 with --raw; the error is printed as JSON, not propagated.
        let result = run(
            &conn, "?invalid", None, None, 10, false, true, true, None, None, None, None,
        );
        assert!(
            result.is_ok(),
            "--raw --json with bad FTS5 must return Ok (error JSON on stdout): {result:?}"
        );
    }
}
