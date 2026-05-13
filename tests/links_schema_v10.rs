#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test fixtures legitimately panic on setup failure"
)]

//! Schema-level tests for the v10 `links` table:
//!   - extended `source_kind` CHECK constraint accepts derived kinds,
//!   - invalid `source_kind` values fail the CHECK,
//!   - manual `programmatic` temporal duplicates remain allowed (no derived
//!     uniqueness applies to programmatic rows),
//!   - the partial unique index `idx_links_unique_derived_edge` rejects
//!     duplicate derived edges per `(from, to, relationship, source_kind)`,
//!   - graph config defaults are seeded at init.

use quaid::core::db::open;
use rusqlite::{params, Connection};

fn seed_two_pages(conn: &Connection) -> (i64, i64) {
    let collection_id: i64 = conn
        .query_row(
            "SELECT id FROM collections ORDER BY id LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let insert = |slug: &str| -> i64 {
        conn.execute(
            "INSERT INTO pages
                 (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline,
                  frontmatter, wing, room, version)
             VALUES (?1, ?2, ?3, 'concept', ?2, '', '', '', '{}', 'notes', '', 1)",
            params![collection_id, slug, uuid::Uuid::now_v7().to_string()],
        )
        .unwrap();
        conn.last_insert_rowid()
    };
    (insert("notes/from"), insert("notes/to"))
}

#[test]
fn fresh_v10_links_check_accepts_all_four_source_kinds() {
    let conn = open(":memory:").unwrap();
    let (from, to) = seed_two_pages(&conn);

    for kind in ["wiki_link", "programmatic", "frontmatter", "entity_pattern"] {
        conn.execute(
            "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind)
             VALUES (?1, ?2, 'related', ?3)",
            params![from, to, kind],
        )
        .unwrap_or_else(|err| panic!("source_kind {kind} should be accepted: {err}"));
    }
}

#[test]
fn fresh_v10_links_check_rejects_invalid_source_kind() {
    let conn = open(":memory:").unwrap();
    let (from, to) = seed_two_pages(&conn);

    let err = conn
        .execute(
            "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind)
             VALUES (?1, ?2, 'related', 'not_a_real_source')",
            params![from, to],
        )
        .expect_err("invalid source_kind should fail CHECK");

    assert!(matches!(err, rusqlite::Error::SqliteFailure(_, _)));
}

#[test]
fn fresh_v10_links_default_edge_weight_is_one_point_zero() {
    let conn = open(":memory:").unwrap();
    let (from, to) = seed_two_pages(&conn);
    conn.execute(
        "INSERT INTO links (from_page_id, to_page_id, relationship)
         VALUES (?1, ?2, 'related')",
        params![from, to],
    )
    .unwrap();

    let weight: f64 = conn
        .query_row(
            "SELECT edge_weight FROM links WHERE from_page_id = ?1 AND to_page_id = ?2",
            params![from, to],
            |row| row.get(0),
        )
        .unwrap();
    assert!((weight - 1.0).abs() < f64::EPSILON);
}

#[test]
fn fresh_v10_links_allows_programmatic_temporal_duplicates() {
    let conn = open(":memory:").unwrap();
    let (from, to) = seed_two_pages(&conn);

    conn.execute(
        "INSERT INTO links
             (from_page_id, to_page_id, relationship, source_kind, valid_from, valid_until)
         VALUES (?1, ?2, 'works_at', 'programmatic', '2020-01-01', '2021-12-31')",
        params![from, to],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO links
             (from_page_id, to_page_id, relationship, source_kind, valid_from, valid_until)
         VALUES (?1, ?2, 'works_at', 'programmatic', '2022-01-01', NULL)",
        params![from, to],
    )
    .unwrap();

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM links
             WHERE from_page_id = ?1 AND to_page_id = ?2
               AND relationship = 'works_at' AND source_kind = 'programmatic'",
            params![from, to],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn fresh_v10_links_partial_unique_index_rejects_duplicate_derived_edges() {
    let conn = open(":memory:").unwrap();
    let (from, to) = seed_two_pages(&conn);

    for kind in ["wiki_link", "frontmatter", "entity_pattern"] {
        conn.execute(
            "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind)
             VALUES (?1, ?2, 'related', ?3)",
            params![from, to, kind],
        )
        .unwrap_or_else(|err| panic!("first insert for {kind} should succeed: {err}"));

        let err = conn
            .execute(
                "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind)
                 VALUES (?1, ?2, 'related', ?3)",
                params![from, to, kind],
            )
            .expect_err(&format!(
                "second insert for {kind} should violate idx_links_unique_derived_edge"
            ));
        assert!(matches!(err, rusqlite::Error::SqliteFailure(_, _)));
    }
}

#[test]
fn fresh_v10_links_partial_unique_index_excludes_programmatic_kind() {
    let conn = open(":memory:").unwrap();
    let (from, to) = seed_two_pages(&conn);

    // Two identical programmatic rows (no temporal differentiation) — partial
    // index excludes 'programmatic' so both must persist.
    conn.execute(
        "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind)
         VALUES (?1, ?2, 'related', 'programmatic')",
        params![from, to],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO links (from_page_id, to_page_id, relationship, source_kind)
         VALUES (?1, ?2, 'related', 'programmatic')",
        params![from, to],
    )
    .unwrap();

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM links
             WHERE from_page_id = ?1 AND to_page_id = ?2 AND source_kind = 'programmatic'",
            params![from, to],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn fresh_v10_seeds_graph_config_defaults() {
    let conn = open(":memory:").unwrap();
    let mut rows: Vec<(String, String)> = conn
        .prepare(
            "SELECT key, value FROM config
             WHERE key IN (
                 'graph_depth',
                 'graph_distance_decay',
                 'graph_expansion_max',
                 'edge_weight_frontmatter',
                 'edge_weight_entity_pattern',
                 'edge_weight_wikilink'
             )
             ORDER BY key",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    rows.sort();
    assert_eq!(
        rows,
        vec![
            ("edge_weight_entity_pattern".to_string(), "0.7".to_string()),
            ("edge_weight_frontmatter".to_string(), "1.0".to_string()),
            ("edge_weight_wikilink".to_string(), "0.5".to_string()),
            ("graph_depth".to_string(), "1".to_string()),
            ("graph_distance_decay".to_string(), "0.5".to_string()),
            ("graph_expansion_max".to_string(), "50".to_string()),
        ]
    );
}
