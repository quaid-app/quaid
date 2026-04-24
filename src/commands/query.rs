use crate::core::gaps;
use crate::core::progressive::progressive_retrieve;
use crate::core::types::SearchResult;
use anyhow::Result;
use rusqlite::Connection;

use crate::core::search::hybrid_search_canonical;

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

pub async fn run(
    db: &Connection,
    query: &str,
    depth: &str,
    limit: u32,
    token_budget: u32,
    wing: Option<String>,
    json: bool,
) -> Result<()> {
    let results = hybrid_search_canonical(query, wing.as_deref(), db, limit as usize)?;

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
        progressive_retrieve(results.clone(), budget, 3, db).unwrap_or(results)
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
    fn low_result_query_auto_logs_gap() {
        use crate::core::db;
        use crate::core::gaps;

        let conn = db::open(":memory:").expect("open db");

        // Query with no results should log a gap
        let results = crate::core::search::hybrid_search_canonical(
            "nonexistent quantum socks",
            None,
            &conn,
            10,
        )
        .unwrap();
        assert!(results.len() < 2);

        // Simulate the gap logging that query::run does
        if results.len() < 2 || results.iter().all(|r| r.score < 0.3) {
            gaps::log_gap(
                None,
                "nonexistent quantum socks",
                "",
                results.first().map(|r| r.score),
                &conn,
            )
            .unwrap();
        }

        let gaps = gaps::list_gaps(false, 10, &conn).unwrap();
        assert_eq!(gaps.len(), 1);
    }
}
