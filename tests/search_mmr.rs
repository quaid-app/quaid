#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure; per-site #[expect] would generate noise across thousands of test sites"
)]

//! MMR reranking tests (`mmr-reranking` capability, openspec change
//! `retrieval-quality-rerank` task 5.7).
//!
//! Scenarios tested:
//!   1. Diversity penalty downranks a near-duplicate behind a more diverse,
//!      lower-scoring candidate (the design.md worked example)
//!   2. First selection equals the top fused-score candidate regardless of λ
//!   3. Deterministic tie-break: tied MMR scores break by page_id ascending
//!   4. `λ = 1.0` reproduces the relevance ordering bytewise (identity)
//!   5. Missing-vector candidates fall through with zero diversity penalty
//!
//! `apply_mmr` reads each candidate's representative embedding back from the
//! `page_embeddings_vec_*` table, so the tests insert synthetic unit vectors
//! directly (no embedding model required) and control pairwise cosine through
//! the angle between two active dimensions.

use quaid::commands::embed;
use quaid::core::db;
use quaid::core::inference::{embedding_to_blob, resolve_model};
use quaid::core::search::{apply_mmr, hybrid_search, HybridSearch};
use quaid::core::types::SearchResult;
use rusqlite::Connection;

const DIM: usize = 384;

fn open_db() -> Connection {
    // Pin the small BGE model (384d) so the synthetic `vec_384` fixtures match
    // the active model's vec table, independent of the production default
    // (Qwen3-Embedding-0.6B, 1024d).
    db::init(":memory:", &resolve_model("small")).expect("init in-memory db")
}

/// Build a 384-dimension unit vector lying at `angle_deg` from the x-axis in
/// the x/y plane (all other dimensions zero). Cosine similarity between two
/// such vectors equals the cosine of the angle between them.
fn unit_vec(angle_deg: f64) -> Vec<f32> {
    let radians = angle_deg.to_radians();
    let mut v = vec![0.0f32; DIM];
    v[0] = radians.cos() as f32;
    v[1] = radians.sin() as f32;
    v
}

/// Insert a page plus a single synthetic chunk embedding. When `embedding` is
/// `None`, the page has no stored vector (missing-vector fall-through path).
fn insert_page_with_vec(conn: &Connection, slug: &str, embedding: Option<&[f32]>) {
    conn.execute(
        "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES (?1, 'concept', ?1, ?1, '', '', '{}', 'notes', '', 1)",
        rusqlite::params![slug],
    )
    .expect("insert page");
    let page_id: i64 = conn
        .query_row("SELECT id FROM pages WHERE slug = ?1", [slug], |row| {
            row.get(0)
        })
        .expect("page id");

    if let Some(vector) = embedding {
        // The vec rowid is allocated from the embedding metadata rowcount so
        // multiple pages get distinct rowids.
        let vec_rowid: i64 = page_id;
        conn.execute(
            "INSERT INTO page_embeddings_vec_384(rowid, embedding) VALUES (?1, ?2)",
            rusqlite::params![vec_rowid, embedding_to_blob(vector)],
        )
        .expect("insert vec row");
        conn.execute(
            "INSERT INTO page_embeddings \
                 (page_id, model, vec_rowid, chunk_type, chunk_index, chunk_text, \
                  content_hash, token_count, heading_path) \
             VALUES (?1, 'BAAI/bge-small-en-v1.5', ?2, 'truth_section', 0, ?3, 'hash', 2, '')",
            rusqlite::params![page_id, vec_rowid, slug],
        )
        .expect("insert embedding metadata");
    }
}

fn candidate(slug: &str, score: f64) -> SearchResult {
    SearchResult {
        slug: slug.to_owned(),
        title: slug.to_owned(),
        summary: slug.to_owned(),
        score,
        wing: "notes".to_owned(),
        ..Default::default()
    }
}

fn slugs(results: &[SearchResult]) -> Vec<&str> {
    results.iter().map(|result| result.slug.as_str()).collect()
}

#[test]
fn diversity_penalty_downranks_near_duplicate() {
    let conn = open_db();
    // c1 at 0°, c2 at ~18.19° (cos ≈ 0.95 to c1), c3 at 90° (cos = 0 to c1).
    insert_page_with_vec(&conn, "c1", Some(&unit_vec(0.0)));
    insert_page_with_vec(&conn, "c2", Some(&unit_vec(18.19)));
    insert_page_with_vec(&conn, "c3", Some(&unit_vec(90.0)));

    let candidates = vec![
        candidate("c1", 0.80),
        candidate("c2", 0.79),
        candidate("c3", 0.60),
    ];

    let reranked = apply_mmr(&conn, candidates, 0.7, 3);

    // c1 selected first (top score), then c3 (diverse) before c2 (near-dup).
    assert_eq!(slugs(&reranked), vec!["c1", "c3", "c2"]);
}

#[test]
fn first_selection_equals_top_fused_score() {
    let conn = open_db();
    insert_page_with_vec(&conn, "low", Some(&unit_vec(0.0)));
    insert_page_with_vec(&conn, "top", Some(&unit_vec(45.0)));

    // Even with a small λ (diversity-heavy), the first pick has an empty
    // `selected` set, so it is the highest fused score.
    let candidates = vec![candidate("low", 0.40), candidate("top", 0.95)];
    let reranked = apply_mmr(&conn, candidates, 0.1, 2);

    assert_eq!(reranked[0].slug, "top");
}

#[test]
fn lambda_one_reproduces_relevance_ordering_bytewise() {
    let conn = open_db();
    insert_page_with_vec(&conn, "a", Some(&unit_vec(0.0)));
    insert_page_with_vec(&conn, "b", Some(&unit_vec(5.0)));
    insert_page_with_vec(&conn, "c", Some(&unit_vec(10.0)));

    let input = vec![
        candidate("a", 0.9),
        candidate("b", 0.8),
        candidate("c", 0.7),
    ];
    let reranked = apply_mmr(&conn, input, 1.0, 3);

    // Identity: order preserved and mmr_score left at its inactive default.
    assert_eq!(slugs(&reranked), vec!["a", "b", "c"]);
    assert!(
        reranked.iter().all(|result| result.mmr_score.is_none()),
        "λ = 1.0 must leave mmr_score unset (identity no-op)"
    );
}

#[test]
fn tied_mmr_scores_break_by_page_id_ascending() {
    let conn = open_db();
    // Two candidates with identical fused score and identical (orthogonal)
    // geometry → identical MMR; the lower page_id must win. Insert "later"
    // first so it gets the smaller id, proving the tie-break is by id, not
    // input order.
    insert_page_with_vec(&conn, "later", Some(&unit_vec(0.0)));
    insert_page_with_vec(&conn, "earlier", Some(&unit_vec(90.0)));

    // Present them in the opposite order of their page ids.
    let candidates = vec![candidate("earlier", 0.5), candidate("later", 0.5)];
    let reranked = apply_mmr(&conn, candidates, 0.5, 2);

    // "later" has the smaller page_id (inserted first), so it is selected
    // first on the tie.
    assert_eq!(reranked[0].slug, "later");
}

#[test]
fn hybrid_search_lambda_one_reproduces_identity_ordering() {
    // Task 5.6: at the identity default (mmr_lambda = 1.0 plus all other
    // signals at identity), hybrid_search must reproduce pure relevance
    // ordering — mmr_score unset, cross_ref_boost zero — i.e. no behaviour
    // change versus the pre-rerank pipeline.
    let conn = open_db();
    conn.execute(
        "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES ('concepts/rust', 'concept', 'Rust', 'Rust', \
                 'Rust is a systems programming language focused on memory safety.', \
                 '', '{}', 'concepts', '', 1)",
        [],
    )
    .expect("insert page");
    conn.execute(
        "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES ('concepts/go', 'concept', 'Go', 'Go', \
                 'Go is a systems programming language focused on simplicity.', \
                 '', '{}', 'concepts', '', 1)",
        [],
    )
    .expect("insert page");
    embed::run(&conn, None, true, false).expect("embed pages");

    // Explicit λ = 1.0 must equal the config-default (also 1.0) ordering, and
    // leave the rerank metadata at its inactive defaults.
    let results = hybrid_search(
        &conn,
        HybridSearch {
            query: "systems programming language",
            limit: 10,
            mmr_lambda: Some(1.0),
            ..Default::default()
        },
    )
    .expect("hybrid search");

    assert!(!results.is_empty(), "query should match the seeded pages");
    assert!(
        results.iter().all(|r| r.mmr_score.is_none()),
        "λ = 1.0 must leave mmr_score unset"
    );
    assert!(
        results.iter().all(|r| r.cross_ref_boost == 0.0),
        "identity config must leave cross_ref_boost at 0.0"
    );
    // Ordering is non-increasing by score (pure relevance ordering).
    for pair in results.windows(2) {
        assert!(
            pair[0].score >= pair[1].score,
            "results must be in non-increasing score order: {} then {}",
            pair[0].score,
            pair[1].score
        );
    }
}

#[test]
fn missing_vector_candidate_falls_through_with_zero_penalty() {
    let conn = open_db();
    insert_page_with_vec(&conn, "vec1", Some(&unit_vec(0.0)));
    // "novec" has no stored embedding.
    insert_page_with_vec(&conn, "novec", None);

    // Lower fused score but no diversity penalty (no vector). It must still be
    // selected without erroring; its MMR score equals λ * fused_score.
    let candidates = vec![candidate("vec1", 0.9), candidate("novec", 0.4)];
    let reranked = apply_mmr(&conn, candidates, 0.7, 2);

    assert_eq!(reranked.len(), 2);
    let novec = reranked
        .iter()
        .find(|result| result.slug == "novec")
        .expect("novec present");
    let expected = 0.7 * 0.4;
    assert!(
        (f64::from(novec.mmr_score.expect("mmr score set")) - expected).abs() < 1e-5,
        "missing-vector MMR score should equal λ * fused_score = {expected}, got {:?}",
        novec.mmr_score
    );
}
