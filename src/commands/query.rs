#![expect(
    clippy::print_stdout,
    reason = "CLI command prints user-facing output to stdout by design"
)]

use crate::core::gaps;
use crate::core::progressive::progressive_retrieve_with_namespace;
use crate::core::types::SearchResult;
use anyhow::Result;
use rusqlite::Connection;

use crate::core::search::{hybrid_search, HybridSearch};

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

#[expect(
    clippy::too_many_arguments,
    reason = "query command CLI accepts the documented user-facing flags directly; collapsing into a struct would obscure the dispatch boundary"
)]
pub async fn run(
    db: &Connection,
    query: &str,
    depth: &str,
    limit: u32,
    token_budget: u32,
    wing: Option<String>,
    namespace: Option<&str>,
    include_superseded: bool,
    json: bool,
    hops: Option<u32>,
    relevance_floor: Option<f64>,
    max_chunks_per_doc: Option<usize>,
    mmr_lambda: Option<f64>,
) -> Result<()> {
    crate::core::namespace::validate_optional_namespace(namespace)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let namespace = namespace.or(Some(""));
    let results = hybrid_search(
        db,
        HybridSearch {
            query,
            wing: wing.as_deref(),
            namespace,
            include_superseded,
            canonical: true,
            limit: limit as usize,
            hops,
            relevance_floor,
            max_chunks_per_doc,
            mmr_lambda,
            ..Default::default()
        },
    )?;

    // Auto-log knowledge gap on weak results
    if gaps::should_log_gap(&results) {
        let context = gaps::auto_gap_context("hybrid_search", &results);
        if let Err(e) = gaps::log_gap(None, query, &context, results.first().map(|r| r.score), db) {
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
        progressive_retrieve_with_namespace(
            results.clone(),
            budget,
            3,
            None,
            namespace,
            include_superseded,
            db,
        )
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

/// Trim `results` to at most `limit` entries fitting `token_budget`.
///
/// Token accounting mirrors `core::progressive`: one token ≈ four characters
/// of rendered output (`<slug>: <summary>` per line). Costs and the final
/// truncation are both measured in characters (not bytes), so multibyte
/// summaries are budgeted consistently and never split mid-character.
fn budget_results(
    results: Vec<SearchResult>,
    limit: usize,
    token_budget: usize,
) -> Vec<SearchResult> {
    let mut remaining_chars = token_budget.saturating_mul(4);
    let mut budgeted = Vec::new();

    for result in results.into_iter().take(limit) {
        // "<slug>: " prefix
        let prefix_chars = result.slug.chars().count() + 2;
        let full_line_chars = prefix_chars + result.summary.chars().count();

        if full_line_chars <= remaining_chars {
            remaining_chars -= full_line_chars;
            budgeted.push(result);
            continue;
        }

        if remaining_chars <= prefix_chars {
            break;
        }

        let summary_budget = remaining_chars - prefix_chars;
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
            ..Default::default()
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

        // 5 tokens = 20 chars; prefix "people/alice: " = 14 chars → 6 summary chars.
        let budgeted = budget_results(results, 10, 5);

        assert_eq!(budgeted.len(), 1);
        assert_eq!(budgeted[0].summary, "abcdef");
    }

    #[test]
    fn budget_results_counts_multibyte_summaries_in_chars_not_bytes() {
        // 100 three-byte chars: a byte-based budget would overcount this
        // summary threefold and a byte-based truncation could split a char.
        let results = vec![result("people/alice", &"あ".repeat(100))];

        // 10 tokens = 40 chars; prefix = 14 chars → 26 summary chars.
        let budgeted = budget_results(results, 10, 10);

        assert_eq!(budgeted.len(), 1);
        assert_eq!(budgeted[0].summary.chars().count(), 26);
        assert!(budgeted[0].summary.chars().all(|ch| ch == 'あ'));
    }

    #[test]
    fn budget_results_zero_budget_breaks_immediately() {
        let results = vec![result("people/alice", "anything")];
        // prefix chars ("people/alice: ") = 14, budget = 0 → 0 <= 14 → break
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
            false,
            true,
            None,
            None,
            None,
            None,
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
            false,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn run_auto_depth_uses_token_budget() {
        use crate::core::db;
        let conn = db::open(":memory:").unwrap();
        // depth="auto" triggers read_token_budget + progressive_retrieve path
        let result = run(
            &conn, "anything", "auto", 5, 0, None, None, false, false, None, None, None, None,
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn run_with_explicit_token_budget_uses_it() {
        use crate::core::db;
        let conn = db::open(":memory:").unwrap();
        let result = run(
            &conn, "query", "auto", 5, 2000, None, None, false, true, None, None, None, None,
        )
        .await;
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
        let result = run(
            &conn,
            "xqzfoo xqzbar",
            "none",
            5,
            10_000,
            None,
            None,
            false,
            false,
            None,
            None,
            None,
            None,
        )
        .await;
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
        let result = run(
            &conn,
            "xqzjson xqzbaz",
            "none",
            5,
            10_000,
            None,
            None,
            false,
            true,
            None,
            None,
            None,
            None,
        )
        .await;
        assert!(result.is_ok());
    }
}
