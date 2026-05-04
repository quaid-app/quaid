use anyhow::Result;
use rusqlite::Connection;

use crate::core::fts::{
    sanitize_fts_query, search_fts_canonical_tiered_with_namespace_filtered,
    search_fts_canonical_with_namespace_filtered,
};

#[allow(clippy::too_many_arguments)]
pub fn run(
    db: &Connection,
    query: &str,
    wing: Option<String>,
    namespace: Option<&str>,
    limit: u32,
    include_superseded: bool,
    json: bool,
    raw: bool,
) -> Result<()> {
    crate::core::namespace::validate_optional_namespace(namespace)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let namespace = namespace.or(Some(""));
    let effective_query = if raw {
        query.to_owned()
    } else {
        sanitize_fts_query(query)
    };
    let results = if raw {
        search_fts_canonical_with_namespace_filtered(
            &effective_query,
            wing.as_deref(),
            None,
            namespace,
            include_superseded,
            db,
            limit as usize,
        )
    } else {
        search_fts_canonical_tiered_with_namespace_filtered(
            &effective_query,
            wing.as_deref(),
            None,
            namespace,
            include_superseded,
            db,
            limit as usize,
        )
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
        let result = run(&conn, "?invalid", None, None, 10, false, true, true);
        assert!(
            result.is_ok(),
            "--raw --json with bad FTS5 must return Ok (error JSON on stdout): {result:?}"
        );
    }
}
