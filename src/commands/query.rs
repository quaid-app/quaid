use crate::core::gaps;
use crate::core::progressive::progressive_retrieve_with_namespace;
use crate::core::types::SearchResult;
use anyhow::Result;
use rusqlite::Connection;

use crate::core::search::hybrid_search_canonical_with_namespace;

/// Read `default_token_budget` from the config table, falling back to 4000.
fn read_token_budget(db: &Connection) -> usize {
    db.query_row(
        "SELECT value FROM config WHERE key = 'default_token_budget'",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|v| v.parse::<usize>().ok())
    .unwrap_or(4000)
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    db: &Connection,
    query: &str,
    depth: &str,
    limit: u32,
    token_budget: u32,
    wing: Option<String>,
    namespace: Option<&str>,
    json: bool,
) -> Result<()> {
    crate::core::namespace::validate_optional_namespace(namespace)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let namespace = namespace.or(Some(""));
    let results = hybrid_search_canonical_with_namespace(
        query,
        wing.as_deref(),
        None,
        namespace,
        db,
        limit as usize,
    )?;

    // Auto-log knowledge gap on weak results
    if results.len() < 2 || results.iter().all(|r| r.score < 0.3) {
        if let Err(e) = gaps::log_gap(None, query, "", results.first().map(|r| r.score), db) {
            eprintln!("Warning: failed to log knowledge gap: {e}");
        } else {
            eprintln!("Knowledge gap logged.");
        }
    }

    let results = if depth == "auto" {
        let budget = if token_budget > 0 {
            token_budget as usize
        } else {
            read_token_budget(db)
        };
        progressive_retrieve_with_namespace(results.clone(), budget, 3, None, namespace, db)
            .unwrap_or(results)
    } else {
        results
    };

    let results = budget_results(results, limit as usize, token_budget as usize);

    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else if results.is_empty() {
        println!("No results found.");
    } else {
        for result in &results {
            println!("{}: {}", result.slug, result.summary);
        }
    }

    Ok(())
}

fn budget_results(
    results: Vec<SearchResult>,
    limit: usize,
    token_budget: usize,
) -> Vec<SearchResult> {
    let mut remaining = token_budget;
    let mut budgeted = Vec::new();

    for result in results.into_iter().take(limit) {
        let line_prefix = format!("{}: ", result.slug);
        let full_line_len = line_prefix.len() + result.summary.len();

        if full_line_len <= remaining {
            remaining = remaining.saturating_sub(full_line_len);
            budgeted.push(result);
            continue;
        }

        if remaining <= line_prefix.len() {
            break;
        }

        let summary_budget = remaining - line_prefix.len();
        let mut truncated = result;
        truncated.summary = truncated.summary.chars().take(summary_budget).collect();
        budgeted.push(truncated);
        break;
    }

    budgeted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(slug: &str, summary: &str) -> SearchResult {
        SearchResult {
            slug: slug.to_owned(),
            title: slug.to_owned(),
            summary: summary.to_owned(),
            score: 1.0,
            wing: "people".to_owned(),
        }
    }

    #[test]
    fn budget_results_applies_limit_before_budgeting() {
        let results = vec![
            result("people/alice", "first"),
            result("people/bob", "second"),
            result("people/carol", "third"),
        ];

        let budgeted = budget_results(results, 2, 1_000);

        assert_eq!(budgeted.len(), 2);
        assert_eq!(budgeted[0].slug, "people/alice");
        assert_eq!(budgeted[1].slug, "people/bob");
    }

    #[test]
    fn budget_results_truncates_summary_to_fit_remaining_budget() {
        let results = vec![result("people/alice", "abcdefghijklmnopqrstuvwxyz")];
        let prefix_len = "people/alice: ".len();

        let budgeted = budget_results(results, 10, prefix_len + 5);

        assert_eq!(budgeted.len(), 1);
        assert_eq!(budgeted[0].summary, "abcde");
    }

    #[test]
    fn budget_results_zero_budget_breaks_immediately() {
        let results = vec![result("people/alice", "anything")];
        // prefix len ("people/alice: ") = 14, budget = 0 → 0 <= 14 → break
        let budgeted = budget_results(results, 10, 0);
        assert!(budgeted.is_empty());
    }

    #[test]
    fn read_token_budget_returns_default_4000_from_config_table() {
        use crate::core::db;
        let conn = db::open(":memory:").unwrap();
        // The schema seeds config with default_token_budget = 4000
        let budget = super::read_token_budget(&conn);
        assert_eq!(budget, 4000);
    }

    #[test]
    fn read_token_budget_returns_custom_value_from_config_table() {
        use crate::core::db;
        let conn = db::open(":memory:").unwrap();
        // config table exists in schema with default 4000; update it to a custom value
        conn.execute(
            "UPDATE config SET value = '8192' WHERE key = 'default_token_budget'",
            [],
        )
        .unwrap();
        let budget = super::read_token_budget(&conn);
        assert_eq!(budget, 8192);
    }

    #[tokio::test]
    async fn run_json_mode_returns_ok_with_empty_results() {
        use crate::core::db;
        let conn = db::open(":memory:").unwrap();
        let result = run(
            &conn,
            "nonexistent zebra query",
            "none",
            5,
            1000,
            None,
            None,
            true,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn run_text_mode_no_results_prints_nothing_and_returns_ok() {
        use crate::core::db;
        let conn = db::open(":memory:").unwrap();
        let result = run(
            &conn,
            "nonexistent zebra query",
            "none",
            5,
            1000,
            None,
            None,
            false,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn run_auto_depth_uses_token_budget() {
        use crate::core::db;
        let conn = db::open(":memory:").unwrap();
        // depth="auto" triggers read_token_budget + progressive_retrieve path
        let result = run(&conn, "anything", "auto", 5, 0, None, None, false).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn run_with_explicit_token_budget_uses_it() {
        use crate::core::db;
        let conn = db::open(":memory:").unwrap();
        let result = run(&conn, "query", "auto", 5, 2000, None, None, true).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn run_text_mode_with_fts_match_prints_results() {
        use crate::core::db;
        let conn = db::open(":memory:").unwrap();
        // Insert a page whose title contains a unique multi-word phrase.
        // FTS5 trigger fires automatically on INSERT.
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, wing, version) \
             VALUES ('concept/xqztest', 'concept', 'xqzfoo xqzbar unique coverage probe', \
                     'An xqzfoo probe page for coverage', '', '', 'concept', 1)",
            [],
        )
        .unwrap();
        // Multi-word query → exact_slug_query returns None → FTS5 path → finds the page
        let result = run(&conn, "xqzfoo xqzbar", "none", 5, 10_000, None, None, false).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn run_json_mode_with_fts_match_serializes_results_array() {
        use crate::core::db;
        let conn = db::open(":memory:").unwrap();
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, wing, version) \
             VALUES ('concept/xqzjson', 'concept', 'xqzjson xqzbaz unique json probe', \
                     'JSON output path probe', '', '', 'concept', 1)",
            [],
        )
        .unwrap();
        let result = run(&conn, "xqzjson xqzbaz", "none", 5, 10_000, None, None, true).await;
        assert!(result.is_ok());
    }
}
