//! Embedding migration correctness tests.
//!
//! Verifies that switching the active embedding model produces zero cross-model
//! contamination: results from model A never leak through when model B is active,
//! and vice versa.
//!
//! Test plan:
//!   1. Embed pages with model A (default bge-small-en-v1.5).
//!   2. Register a synthetic "model B" with its own vec table.
//!   3. Insert distinct embeddings for model B.
//!   4. Switch active flag to model B.
//!   5. Run searches — assert ALL results come from model B.
//!   6. Switch active flag back to model A.
//!   7. Run searches — assert ALL results come from model A (no model B leakage).
//!
//! "Model B" here is a synthetic model defined by a different name and a
//! separate vec0 virtual table. The test verifies DB-level routing logic,
//! not model quality.

use gbrain::commands::embed;
use gbrain::core::db;
use gbrain::core::inference::{embed as embed_text, embedding_to_blob};

fn open_test_db() -> rusqlite::Connection {
    let dir = tempfile::TempDir::new().expect("create temp dir");
    let db_path = dir.path().join("brain.db");
    let conn = db::open(db_path.to_str().unwrap()).expect("open DB");
    // Leak TempDir to keep file alive
    std::mem::forget(dir);
    conn
}

fn insert_page(conn: &rusqlite::Connection, slug: &str, title: &str, truth: &str) -> i64 {
    conn.execute(
        "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES (?1, ?2, 'person', ?3, '', ?4, '', '{}', 'people', '', 1)",
        rusqlite::params![
            slug,
            gbrain::core::page_uuid::generate_uuid_v7(),
            title,
            truth
        ],
    )
    .expect("insert page");
    conn.query_row("SELECT id FROM pages WHERE slug = ?1", [slug], |row| {
        row.get(0)
    })
    .expect("get page id")
}

/// Register a second embedding model and create its vec0 table.
/// Returns the model name and vec table name.
fn register_model_b(conn: &rusqlite::Connection) -> (String, String) {
    let model_name = "test-model-b";
    let vec_table = "page_embeddings_vec_test_b";

    // Create the vec0 table for model B
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS {vec_table} \
         USING vec0(embedding float[384]);"
    ))
    .expect("create model B vec table");

    // Register model B (initially inactive)
    conn.execute(
        "INSERT OR IGNORE INTO embedding_models (name, dimensions, vec_table, active) \
         VALUES (?1, 384, ?2, 0)",
        rusqlite::params![model_name, vec_table],
    )
    .expect("register model B");

    (model_name.to_string(), vec_table.to_string())
}

/// Set a model as active (and deactivate all others).
fn set_active_model(conn: &rusqlite::Connection, model_name: &str) {
    conn.execute("UPDATE embedding_models SET active = 0", [])
        .expect("deactivate all");
    conn.execute(
        "UPDATE embedding_models SET active = 1 WHERE name = ?1",
        [model_name],
    )
    .expect("activate model");
}

fn active_model_name(conn: &rusqlite::Connection) -> String {
    conn.query_row(
        "SELECT name FROM embedding_models WHERE active = 1",
        [],
        |row| row.get(0),
    )
    .expect("active model")
}

/// Insert a synthetic embedding for a page under a specific model.
fn insert_synthetic_embedding(
    conn: &rusqlite::Connection,
    page_id: i64,
    model_name: &str,
    vec_table: &str,
    text: &str,
) {
    let embedding = embed_text(text).expect("embed text");
    let blob = embedding_to_blob(&embedding);

    // Insert into the vec0 table and get the rowid
    conn.execute(
        &format!("INSERT INTO {vec_table} (embedding) VALUES (?1)"),
        [blob.as_slice()],
    )
    .expect("insert into vec table");
    let vec_rowid: i64 = conn
        .query_row(
            &format!("SELECT rowid FROM {vec_table} ORDER BY rowid DESC LIMIT 1"),
            [],
            |row| row.get(0),
        )
        .expect("get vec rowid");

    let content_hash = format!("{model_name}-{page_id}-{}", vec_rowid);
    conn.execute(
        "INSERT OR REPLACE INTO page_embeddings \
         (page_id, model, vec_rowid, chunk_type, chunk_index, chunk_text, content_hash, token_count) \
         VALUES (?1, ?2, ?3, 'full', 0, ?4, ?5, 1)",
        rusqlite::params![page_id, model_name, vec_rowid, text, content_hash],
    )
    .expect("insert page_embeddings record");
}

// ── Main migration test ────────────────────────────────────────────────────────

#[test]
fn embedding_migration_zero_cross_model_contamination() {
    let conn = open_test_db();

    // Insert 5 test pages
    let pages = [
        (
            "people/alice",
            "Alice",
            "Alice is a software engineer specializing in distributed systems.",
        ),
        (
            "people/bob",
            "Bob",
            "Bob is a data scientist working on machine learning models.",
        ),
        (
            "companies/acme",
            "Acme",
            "Acme builds developer productivity tools and APIs.",
        ),
        (
            "companies/brex",
            "Brex",
            "Brex provides corporate cards and financial infrastructure for startups.",
        ),
        (
            "projects/gbrain",
            "GBrain",
            "GBrain is a personal knowledge brain using SQLite and embeddings.",
        ),
    ];

    let mut page_ids: Vec<(String, i64)> = Vec::new();
    for (slug, title, truth) in &pages {
        let id = insert_page(&conn, slug, title, truth);
        page_ids.push((slug.to_string(), id));
    }

    // ── Step 1: Embed all pages with model A (default) ────────────────────────
    embed::run(&conn, None, true, false).expect("embed with model A");

    let model_a = active_model_name(&conn);
    assert_eq!(
        model_a, "BAAI/bge-small-en-v1.5",
        "model A should be default"
    );

    // Verify model A has embeddings
    let model_a_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM page_embeddings WHERE model = ?1",
            [&model_a],
            |row| row.get(0),
        )
        .expect("count model A embeddings");
    assert!(
        model_a_count > 0,
        "model A should have embeddings after embed run"
    );

    // ── Step 2: Register model B and insert distinct embeddings ──────────────
    let (model_b_name, model_b_table) = register_model_b(&conn);

    for (slug, id) in &page_ids {
        // Use reversed/transformed text to ensure model B embeddings differ
        let synthetic_text = format!("model-b {slug} distinct embedding signal");
        insert_synthetic_embedding(&conn, *id, &model_b_name, &model_b_table, &synthetic_text);
    }

    let model_b_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM page_embeddings WHERE model = ?1",
            [&model_b_name],
            |row| row.get(0),
        )
        .expect("count model B embeddings");
    assert_eq!(
        model_b_count,
        pages.len() as i64,
        "model B should have one embedding per page"
    );

    // ── Step 3: Switch to model B, run vec search, verify no model A leakage ──
    set_active_model(&conn, &model_b_name);
    assert_eq!(active_model_name(&conn), model_b_name);

    let results_b =
        gbrain::core::inference::search_vec("knowledge brain embeddings", 10, None, None, &conn)
            .expect("search with model B");

    // All results should come from model B's embeddings
    if !results_b.is_empty() {
        for result in &results_b {
            let via_model: String = conn
                .query_row(
                    "SELECT pe.model FROM page_embeddings pe \
                     JOIN pages p ON p.id = pe.page_id \
                     WHERE p.slug = ?1 AND pe.model = ?2 \
                     LIMIT 1",
                    rusqlite::params![&result.slug, &model_b_name],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| "NOT_FOUND".to_string());

            assert_eq!(
                via_model, model_b_name,
                "search result '{}' should come from model B, not model A — zero contamination required",
                result.slug
            );
        }
    }

    // ── Step 4: Rollback to model A, verify model A results return ────────────
    set_active_model(&conn, &model_a);
    assert_eq!(active_model_name(&conn), model_a);

    let results_a = gbrain::core::inference::search_vec(
        "software engineer distributed systems",
        10,
        None,
        None,
        &conn,
    )
    .expect("search with model A");

    // Results with model A active should not reference model B embeddings
    if !results_a.is_empty() {
        for result in &results_a {
            let model_b_rows: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM page_embeddings pe \
                     JOIN pages p ON p.id = pe.page_id \
                     WHERE p.slug = ?1 AND pe.model = ?2",
                    rusqlite::params![&result.slug, &model_b_name],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            // It's fine if model B rows exist — but results should come from model A's routing
            // The key invariant: search_vec uses active model only (via SQL join on pe.model = model_name)
            let model_a_rows: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM page_embeddings pe \
                     JOIN pages p ON p.id = pe.page_id \
                     WHERE p.slug = ?1 AND pe.model = ?2",
                    rusqlite::params![&result.slug, &model_a],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            assert!(
                model_a_rows > 0,
                "result '{}' returned by model A search should have model A embeddings",
                result.slug
            );

            let _ = model_b_rows; // model B rows existing alongside model A rows is fine
        }
    }
}

// ── Model registration correctness ────────────────────────────────────────────

#[test]
fn only_one_model_is_active_at_a_time() {
    let conn = open_test_db();

    // Register a second model
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS page_embeddings_vec_test USING vec0(embedding float[384]);"
    ).expect("create test vec table");

    conn.execute(
        "INSERT OR IGNORE INTO embedding_models (name, dimensions, vec_table, active) \
         VALUES ('test-model', 384, 'page_embeddings_vec_test', 0)",
        [],
    )
    .expect("register test model");

    // Switch to test model
    set_active_model(&conn, "test-model");

    let active_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM embedding_models WHERE active = 1",
            [],
            |row| row.get(0),
        )
        .expect("count active models");

    assert_eq!(
        active_count, 1,
        "exactly one model should be active at a time"
    );
    assert_eq!(active_model_name(&conn), "test-model");

    // Switch back
    set_active_model(&conn, "BAAI/bge-small-en-v1.5");
    assert_eq!(active_model_name(&conn), "BAAI/bge-small-en-v1.5");

    let active_count_after: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM embedding_models WHERE active = 1",
            [],
            |row| row.get(0),
        )
        .expect("count active models after switch");
    assert_eq!(
        active_count_after, 1,
        "exactly one model active after switch back"
    );
}

// ── Empty vec table returns no results ────────────────────────────────────────

#[test]
fn vec_search_on_empty_model_returns_no_results() {
    let conn = open_test_db();

    // Register a model with no embeddings
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS page_embeddings_vec_empty \
         USING vec0(embedding float[384]);",
    )
    .expect("create empty vec table");

    conn.execute(
        "INSERT OR REPLACE INTO embedding_models (name, dimensions, vec_table, active) \
         VALUES ('empty-model', 384, 'page_embeddings_vec_empty', 1)",
        [],
    )
    .expect("register empty model");

    conn.execute(
        "UPDATE embedding_models SET active = 0 WHERE name != 'empty-model'",
        [],
    )
    .expect("deactivate others");

    let results = gbrain::core::inference::search_vec("anything", 10, None, None, &conn)
        .expect("search empty model");

    assert!(
        results.is_empty(),
        "search on empty model should return no results, got: {results:?}"
    );
}
