#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Property test for the FTS5 query-sanitization invariant: for *any* input
//! string, the natural-language search path must never surface an FTS5
//! `MATCH` syntax error (which would arrive as `SearchError::Sqlite`) and must
//! never panic.
//!
//! `sanitize_fts_query` is `pub(crate)`, so this drives the invariant through
//! the public production entry point `hybrid_search`, which sanitizes the
//! query internally before handing it to FTS5. That is the stronger, more
//! behavioural statement of the guarantee: it is not enough that the sanitizer
//! returns *some* string — that string must be a syntactically valid FTS5
//! `MATCH` expression that SQLite accepts.
//!
//! The DB is seeded with one page but no embeddings, so
//! `search_vec_*` short-circuits to an empty result before touching the
//! embedding model (see `inference.rs`: it returns early when
//! `page_embeddings` is empty for the active model). The proptest therefore
//! exercises the full FTS sanitize-and-match path without loading a model.

use proptest::prelude::*;
use rusqlite::Connection;

use quaid::core::db;
use quaid::core::search::{hybrid_search, HybridSearch};

fn seeded_db() -> Connection {
    let conn = db::open(":memory:").expect("open in-memory DB");
    conn.execute(
        "INSERT INTO pages \
         (slug, type, title, summary, compiled_truth, timeline, frontmatter, \
          wing, room, version, created_at, updated_at, truth_updated_at, timeline_updated_at) \
         VALUES ('people/alice', 'person', 'Alice', 'an engineer', \
                 'Alice is a rust engineer who likes SQLite and FTS5.', '', '{}', \
                 'people', '', 1, \
                 strftime('%Y-%m-%dT%H:%M:%SZ','now'), \
                 strftime('%Y-%m-%dT%H:%M:%SZ','now'), \
                 strftime('%Y-%m-%dT%H:%M:%SZ','now'), \
                 strftime('%Y-%m-%dT%H:%M:%SZ','now'))",
        [],
    )
    .expect("seed page");
    conn
}

fn assert_no_fts_syntax_error(conn: &Connection, query: &str) {
    match hybrid_search(
        conn,
        HybridSearch {
            query,
            limit: 10,
            ..Default::default()
        },
    ) {
        Ok(_) => {}
        // The *only* acceptable failure mode for arbitrary natural-language
        // input is none — an FTS5 MATCH syntax error would arrive here as
        // `SearchError::Sqlite` and is exactly what sanitization must prevent.
        Err(err) => panic!("hybrid_search errored on query {query:?}: {err}"),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    /// Arbitrary UTF-8 strings (including FTS5 operators, quotes, parentheses,
    /// CJK, emoji, combining marks, and the boolean keywords) must never
    /// produce an FTS5 syntax error or panic.
    #[test]
    fn arbitrary_unicode_query_never_errors(query in any::<String>()) {
        let conn = seeded_db();
        assert_no_fts_syntax_error(&conn, &query);
    }

    /// Targeted adversarial shapes: dense runs of FTS5 metacharacters and the
    /// case-sensitive boolean keywords interleaved with junk, which is where a
    /// naive sanitizer leaves a dangling operator.
    #[test]
    fn adversarial_operator_soup_never_errors(
        query in r#"[?*+":()^.\-/@#=, ANDORNTearx]{0,64}"#
    ) {
        let conn = seeded_db();
        assert_no_fts_syntax_error(&conn, &query);
    }
}

/// A regression net of the specific shapes the unit tests in `fts.rs` cover,
/// plus a few that have historically broken naive FTS sanitizers, exercised
/// through the same public path.
#[test]
fn known_fts_breakers_do_not_error() {
    let conn = seeded_db();
    for query in [
        "AND",
        "OR NOT NEAR",
        "\"unterminated",
        "rust AND",
        "(((",
        ")))",
        "* prefix",
        "NEAR(a b)",
        "foo: bar",
        "a^b",
        "\u{4e2d}\u{6587} AND \u{1f600}",
        "   ",
        "",
        "???***",
        "col:val AND NOT \"x",
    ] {
        assert_no_fts_syntax_error(&conn, query);
    }
}
