//! Novelty detection — deduplication check for ingestion.
//!
//! Phase 1 treats novelty as a conservative duplicate filter:
//! - lexical overlap is measured with lowercased whitespace-token Jaccard
//! - stored embeddings are consulted when present for a near-duplicate check
//!
//! The current embedding backend is a deterministic SHA-256 placeholder, not a
//! semantic model, so tests intentionally only lock down duplicate-vs-different
//! behavior and avoid paraphrase expectations.

use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension};

use super::inference::{embed, embedding_to_blob};
use super::types::{Page, SearchError};

const JACCARD_DUPLICATE_THRESHOLD: f64 = 0.85;
const COSINE_DUPLICATE_THRESHOLD: f64 = 0.95;

/// Returns `true` when `content` is novel relative to `existing_page`.
pub fn check_novelty(
    content: &str,
    existing_page: &Page,
    conn: &Connection,
) -> Result<bool, SearchError> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(false);
    }

    if jaccard_similarity(trimmed, &existing_page.compiled_truth) >= JACCARD_DUPLICATE_THRESHOLD {
        return Ok(false);
    }

    let Some((model_name, vec_table)) = active_model(conn)? else {
        return Ok(true);
    };

    if !is_safe_identifier(&vec_table) {
        return Err(SearchError::Internal {
            message: format!("unsafe vec table name: {vec_table}"),
        });
    }

    let query_embedding = embed(trimmed).map_err(|err| SearchError::Internal {
        message: err.to_string(),
    })?;
    let query_blob = embedding_to_blob(&query_embedding);

    let sql = format!(
        "SELECT MAX(1.0 - vec_distance_cosine(pev.embedding, ?1)) \
         FROM {vec_table} pev \
         JOIN page_embeddings pe ON pev.rowid = pe.vec_rowid \
         JOIN pages p ON p.id = pe.page_id \
         WHERE pe.model = ?2 AND p.slug = ?3"
    );

    let max_similarity = conn.query_row(
        &sql,
        params![query_blob, model_name, existing_page.slug.as_str()],
        |row| row.get::<_, Option<f64>>(0),
    )?;

    Ok(max_similarity
        .map(|score| score < COSINE_DUPLICATE_THRESHOLD)
        .unwrap_or(true))
}

fn active_model(conn: &Connection) -> Result<Option<(String, String)>, SearchError> {
    conn.query_row(
        "SELECT name, vec_table FROM embedding_models WHERE active = 1 LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(SearchError::from)
}

fn is_safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn jaccard_similarity(left: &str, right: &str) -> f64 {
    let left_tokens = tokenize(left);
    let right_tokens = tokenize(right);

    if left_tokens.is_empty() && right_tokens.is_empty() {
        return 1.0;
    }

    let intersection = left_tokens.intersection(&right_tokens).count();
    let union = left_tokens.union(&right_tokens).count();

    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

fn tokenize(content: &str) -> HashSet<String> {
    content
        .split_whitespace()
        .map(|token| token.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::collections::HashMap;

    use super::*;
    use crate::core::db;

    fn open_test_db() -> Connection {
        db::open(":memory:").expect("open db")
    }

    fn insert_page(conn: &Connection, slug: &str, compiled_truth: &str) -> Page {
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version) \
             VALUES (?1, 'person', 'Alice', 'Founder', ?2, '', '{}', 'people', '', 1)",
            params![slug, compiled_truth],
        )
        .expect("insert page");

        Page {
            slug: slug.to_owned(),
            page_type: "person".to_owned(),
            title: "Alice".to_owned(),
            summary: "Founder".to_owned(),
            compiled_truth: compiled_truth.to_owned(),
            timeline: String::new(),
            frontmatter: HashMap::new(),
            wing: "people".to_owned(),
            room: String::new(),
            version: 1,
            created_at: "2024-01-01T00:00:00Z".to_owned(),
            updated_at: "2024-01-01T00:00:00Z".to_owned(),
            truth_updated_at: "2024-01-01T00:00:00Z".to_owned(),
            timeline_updated_at: "2024-01-01T00:00:00Z".to_owned(),
        }
    }

    fn insert_embedding(conn: &Connection, page: &Page, chunk_text: &str) {
        let page_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE slug = ?1",
                params![page.slug.as_str()],
                |row| row.get(0),
            )
            .expect("fetch page id");
        let vec_rowid: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(vec_rowid), 0) + 1 FROM page_embeddings",
                [],
                |row| row.get(0),
            )
            .expect("next vec rowid");

        let embedding = embed(chunk_text).expect("embed chunk");
        let blob = embedding_to_blob(&embedding);

        conn.execute(
            "INSERT INTO page_embeddings_vec_384(rowid, embedding) VALUES (?1, ?2)",
            params![vec_rowid, blob],
        )
        .expect("insert vec row");
        conn.execute(
            "INSERT INTO page_embeddings (page_id, model, vec_rowid, chunk_type, chunk_index, chunk_text, content_hash, token_count, heading_path) \
             VALUES (?1, 'BAAI/bge-small-en-v1.5', ?2, 'truth_section', 0, ?3, 'hash', 1, 'State')",
            params![page_id, vec_rowid, chunk_text],
        )
        .expect("insert embedding metadata");
    }

    mod check_novelty {
        use super::*;

        #[test]
        fn identical_content_is_not_novel_even_without_embeddings() {
            let conn = open_test_db();
            let existing = insert_page(
                &conn,
                "people/alice",
                "Alice works at Acme and invests in climate software.",
            );

            let is_novel = check_novelty(
                "Alice works at Acme and invests in climate software.",
                &existing,
                &conn,
            )
            .expect("novelty check succeeds");

            assert!(!is_novel);
        }

        #[test]
        fn near_duplicate_content_is_not_novel_without_embeddings() {
            let conn = open_test_db();
            let existing = insert_page(
                &conn,
                "people/alice",
                "w1 w2 w3 w4 w5 w6 w7 w8 w9 w10 w11 w12 w13 w14 w15 w16 w17 w18 w19 old",
            );

            let is_novel = check_novelty(
                "w1 w2 w3 w4 w5 w6 w7 w8 w9 w10 w11 w12 w13 w14 w15 w16 w17 w18 w19 new",
                &existing,
                &conn,
            )
            .expect("novelty check succeeds");

            assert!(!is_novel);
        }

        #[test]
        fn empty_content_is_not_novel() {
            let conn = open_test_db();
            let existing = insert_page(&conn, "people/alice", "Alice works at Acme.");

            let is_novel = check_novelty("   ", &existing, &conn).expect("novelty check succeeds");

            assert!(!is_novel);
        }

        #[test]
        fn clearly_different_content_is_novel_when_embeddings_are_absent() {
            let conn = open_test_db();
            let existing = insert_page(
                &conn,
                "people/alice",
                "Alice works at Acme and invests in climate software.",
            );

            let is_novel = check_novelty(
                "Bob teaches medieval history and collects rare maps.",
                &existing,
                &conn,
            )
            .expect("novelty check succeeds");

            assert!(is_novel);
        }

        #[test]
        fn clearly_different_content_stays_novel_with_placeholder_embeddings() {
            let conn = open_test_db();
            let existing = insert_page(
                &conn,
                "people/alice",
                "Alice works at Acme and invests in climate software.",
            );
            insert_embedding(
                &conn,
                &existing,
                "Alice works at Acme and invests in climate software.",
            );

            let is_novel = check_novelty(
                "Bob teaches medieval history and collects rare maps.",
                &existing,
                &conn,
            )
            .expect("novelty check succeeds");

            assert!(is_novel);
        }

        #[test]
        fn unsafe_vec_table_name_returns_error() {
            let conn = open_test_db();
            let existing = insert_page(&conn, "people/alice", "Alice works at Acme.");
            conn.execute(
                "UPDATE embedding_models SET vec_table = 'page_embeddings_vec_384;drop' WHERE active = 1",
                [],
            )
            .unwrap();

            let error = check_novelty("Bob writes books.", &existing, &conn).unwrap_err();

            assert!(matches!(error, SearchError::Internal { .. }));
        }
    }

    mod is_safe_identifier {
        #[test]
        fn accepts_alphanumeric_and_underscore_names() {
            assert!(super::is_safe_identifier("page_embeddings_vec_384"));
        }

        #[test]
        fn rejects_punctuation() {
            assert!(!super::is_safe_identifier("page-embeddings"));
        }
    }

    mod jaccard_similarity {
        #[test]
        fn returns_one_for_two_empty_inputs() {
            assert_eq!(super::jaccard_similarity("", ""), 1.0);
        }

        #[test]
        fn returns_zero_when_sets_do_not_overlap() {
            assert_eq!(super::jaccard_similarity("alpha beta", "gamma delta"), 0.0);
        }
    }
}
