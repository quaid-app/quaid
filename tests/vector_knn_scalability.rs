#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "integration tests panic on setup failure and print diagnostics"
)]

//! Integration coverage for review item #10 (vector index scalability):
//!
//! * two-phase KNN retrieval returns bit-for-bit the same ranked top-k and
//!   scores as the brute-force full scan, under every business filter
//!   (collection / namespace / wing / superseded / quarantine);
//! * the larger-than-over-fetch corpus exercises the vec0 `MATCH ... k = ?`
//!   heap path (not just the full-scan fallback);
//! * bulk page-delete paths (`destroy_namespace`, collection purge) drop the
//!   backing vec0 rows so the vec table count returns to zero;
//! * `validate` surfaces orphaned vec rows (vec → page_embeddings dangling).

use quaid::commands::collection::{self, CollectionAction};
use quaid::commands::embed::run_with_batch;
use quaid::core::db;
use quaid::core::inference::search_vec_with_namespace_filtered;
use quaid::core::namespace;
use rusqlite::Connection;
use uuid::Uuid;

fn open_test_db() -> Connection {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    // Keep the temp dir alive for the life of the connection.
    std::mem::forget(dir);
    conn
}

#[allow(clippy::too_many_arguments)]
fn insert_page(
    conn: &Connection,
    slug: &str,
    wing: &str,
    namespace: &str,
    collection_id: i64,
    body: &str,
) {
    conn.execute(
        "INSERT INTO pages
             (slug, uuid, namespace, collection_id, type, title, summary,
              compiled_truth, timeline, frontmatter, wing, room, version)
         VALUES (?1, ?2, ?3, ?4, 'concept', ?1, '', ?5, '', '{}', ?6, '', 1)",
        rusqlite::params![
            slug,
            Uuid::now_v7().to_string(),
            namespace,
            collection_id,
            format!("## State\n{body}"),
            wing,
        ],
    )
    .unwrap();
}

/// `(label, wing, collection, namespace, include_superseded)` for one parity
/// case in [`knn_matches_brute_force_under_every_filter`].
type FilterCase = (
    &'static str,
    Option<&'static str>,
    Option<i64>,
    Option<&'static str>,
    bool,
);

fn vec_row_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM page_embeddings_vec_384", [], |row| {
        row.get(0)
    })
    .unwrap()
}

/// The pre-KNN brute-force query, kept verbatim in the test as the parity
/// oracle. Mirrors `search_vec_internal`'s old full-scan SQL: every chunk is
/// scored with `1.0 - vec_distance_cosine`, then grouped per page.
fn brute_force(
    conn: &Connection,
    query: &str,
    k: usize,
    wing: Option<&str>,
    collection: Option<i64>,
    namespace_filter: Option<&str>,
    include_superseded: bool,
) -> Vec<(String, f64)> {
    let query_blob =
        quaid::core::inference::embedding_to_blob(&quaid::core::inference::embed(query).unwrap());
    let model_name: String = conn
        .query_row(
            "SELECT name FROM embedding_models WHERE active = 1 LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let mut sql = String::from(
        "SELECT p.slug, \
                MAX(1.0 - vec_distance_cosine(pev.embedding, ?1)) AS score \
         FROM page_embeddings_vec_384 pev \
         JOIN page_embeddings pe ON pev.rowid = pe.vec_rowid \
         JOIN pages p ON p.id = pe.page_id \
         WHERE pe.model = ?2 \
           AND p.quarantined_at IS NULL",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> =
        vec![Box::new(query_blob), Box::new(model_name)];
    if let Some(wing) = wing {
        sql.push_str(" AND p.wing = ?3");
        params.push(Box::new(wing.to_owned()));
    }
    if let Some(collection) = collection {
        sql.push_str(&format!(" AND p.collection_id = ?{}", params.len() + 1));
        params.push(Box::new(collection));
    }
    if let Some(ns) = namespace_filter {
        if ns.is_empty() {
            sql.push_str(&format!(" AND p.namespace = ?{}", params.len() + 1));
            params.push(Box::new(String::new()));
        } else {
            sql.push_str(&format!(
                " AND (p.namespace = ?{} OR p.namespace = '')",
                params.len() + 1
            ));
            params.push(Box::new(ns.to_owned()));
        }
    }
    if !include_superseded {
        sql.push_str(" AND p.superseded_by IS NULL");
    }
    sql.push_str(&format!(
        " GROUP BY p.id ORDER BY score DESC LIMIT ?{}",
        params.len() + 1
    ));
    params.push(Box::new(k as i64));

    let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql).unwrap();
    let rows = stmt
        .query_map(refs.as_slice(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })
        .unwrap();
    rows.map(|r| r.unwrap()).collect()
}

fn knn(
    conn: &Connection,
    query: &str,
    k: usize,
    wing: Option<&str>,
    collection: Option<i64>,
    namespace_filter: Option<&str>,
    include_superseded: bool,
) -> Vec<(String, f64)> {
    search_vec_with_namespace_filtered(
        query,
        k,
        wing,
        collection,
        namespace_filter,
        include_superseded,
        conn,
    )
    .unwrap()
    .into_iter()
    .map(|r| (r.slug, r.score))
    .collect()
}

fn assert_parity(actual: &[(String, f64)], expected: &[(String, f64)], context: &str) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "{context}: result count differs\n  knn={actual:?}\n  brute={expected:?}"
    );
    for (i, ((a_slug, a_score), (e_slug, e_score))) in
        actual.iter().zip(expected.iter()).enumerate()
    {
        assert_eq!(
            a_slug, e_slug,
            "{context}: rank {i} slug differs\n  knn={actual:?}\n  brute={expected:?}"
        );
        assert!(
            (a_score - e_score).abs() < 1e-6,
            "{context}: rank {i} score differs ({a_score} vs {e_score})"
        );
    }
}

#[test]
fn knn_matches_brute_force_under_every_filter() {
    let conn = open_test_db();
    // Two collections so the collection filter has something to discriminate.
    conn.execute(
        "INSERT INTO collections (id, name, root_path, writable, is_write_target, state)
         VALUES (2, 'work', '/tmp/work-knn-parity', 1, 0, 'active')",
        [],
    )
    .unwrap();

    let topics = [
        ("neural networks and deep learning architectures", "tech"),
        ("distributed databases and consensus protocols", "tech"),
        ("the history of renaissance oil painting", "art"),
        ("baroque chamber music and counterpoint", "art"),
        ("supply chain logistics optimization", "ops"),
        ("kubernetes scheduling and autoscaling", "ops"),
        ("photosynthesis in marine phytoplankton", "science"),
        ("quantum error correction codes", "science"),
    ];
    for (index, (body, wing)) in topics.iter().enumerate() {
        let collection_id = if index % 2 == 0 { 1 } else { 2 };
        let namespace = if index % 3 == 0 { "alpha" } else { "" };
        insert_page(
            &conn,
            &format!("notes/topic-{index:02}"),
            wing,
            namespace,
            collection_id,
            body,
        );
    }
    // Mark one page superseded and one quarantined to exercise those filters.
    conn.execute(
        "UPDATE pages SET quarantined_at = '2026-01-01T00:00:00Z' WHERE slug = 'notes/topic-03'",
        [],
    )
    .unwrap();
    run_with_batch(&conn, None, true, false, Some(8), false).unwrap();
    // Supersede after embedding so the chunk still exists in the vec table.
    conn.execute(
        "UPDATE pages SET superseded_by = (SELECT id FROM pages WHERE slug='notes/topic-00')
         WHERE slug = 'notes/topic-05'",
        [],
    )
    .unwrap();

    let query = "machine learning systems";
    let cases: Vec<FilterCase> = vec![
        ("no filters", None, None, None, false),
        ("wing=tech", Some("tech"), None, None, false),
        ("collection=2", None, Some(2), None, false),
        ("namespace=alpha", None, None, Some("alpha"), false),
        ("namespace=empty", None, None, Some(""), false),
        ("include_superseded", None, None, None, true),
        ("wing+collection", Some("science"), Some(1), None, false),
    ];
    for (label, wing, collection, ns, superseded) in cases {
        for k in [1usize, 3, 8] {
            let actual = knn(&conn, query, k, wing, collection, ns, superseded);
            let expected = brute_force(&conn, query, k, wing, collection, ns, superseded);
            assert_parity(&actual, &expected, &format!("{label} k={k}"));
        }
    }
}

#[test]
fn knn_heap_path_matches_brute_force_above_overfetch_threshold() {
    // The KNN branch only runs when the over-fetch (max(k*8, 256)) is smaller
    // than the corpus, so seed > 256 chunks to exercise the vec0 heap rather
    // than the full-scan fallback.
    let conn = open_test_db();
    let topics = [
        "alpha vector retrieval",
        "beta semantic ranking",
        "gamma nearest neighbor",
        "delta embedding similarity",
    ];
    for index in 0..300 {
        let body = format!("{} variant {index}", topics[index % topics.len()]);
        insert_page(
            &conn,
            &format!("notes/bulk-{index:03}"),
            "notes",
            "",
            1,
            &body,
        );
    }
    run_with_batch(&conn, None, true, false, Some(64), false).unwrap();
    assert!(
        vec_row_count(&conn) > 256,
        "corpus must exceed the over-fetch floor to hit the KNN heap path"
    );

    let query = "nearest neighbor vector similarity";
    for k in [1usize, 5, 10] {
        let actual = knn(&conn, query, k, None, None, None, false);
        let expected = brute_force(&conn, query, k, None, None, None, false);
        assert_parity(&actual, &expected, &format!("heap-path k={k}"));
    }
}

#[test]
fn destroy_namespace_drops_backing_vec_rows() {
    let conn = open_test_db();
    insert_page(&conn, "notes/keep", "notes", "", 1, "page that survives");
    insert_page(
        &conn,
        "notes/doomed",
        "notes",
        "q0001",
        1,
        "page in doomed ns",
    );
    run_with_batch(&conn, None, true, false, Some(8), false).unwrap();
    let before = vec_row_count(&conn);
    assert!(before >= 2);

    let deleted = namespace::destroy_namespace(&conn, "q0001").unwrap();
    assert_eq!(deleted, 1);

    let remaining = vec_row_count(&conn);
    assert_eq!(
        remaining,
        before - 1,
        "destroying the namespace must drop exactly its page's vec rows"
    );

    // No orphans left behind: every surviving vec row still joins to a page_embeddings row.
    let orphans: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM page_embeddings_vec_384 v \
             WHERE NOT EXISTS (SELECT 1 FROM page_embeddings pe WHERE pe.vec_rowid = v.rowid)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(orphans, 0, "namespace destroy must not orphan vec rows");
}

#[test]
fn collection_purge_zeroes_the_vec_table() {
    let conn = open_test_db();
    let root = tempfile::TempDir::new().unwrap();
    conn.execute(
        "INSERT INTO collections (id, name, root_path, writable, is_write_target, state)
         VALUES (2, 'work', ?1, 1, 0, 'detached')",
        [root.path().to_str().unwrap()],
    )
    .unwrap();
    insert_page(&conn, "notes/work-a", "notes", "", 2, "first work page");
    insert_page(&conn, "notes/work-b", "notes", "", 2, "second work page");
    insert_page(
        &conn,
        "notes/keep",
        "notes",
        "",
        1,
        "page in default collection",
    );
    run_with_batch(&conn, None, true, false, Some(8), false).unwrap();
    let before = vec_row_count(&conn);
    assert!(before >= 3);

    collection::run(
        &conn,
        CollectionAction::Remove {
            name: "work".to_owned(),
            purge: true,
            confirm: true,
        },
        true,
    )
    .unwrap();

    // The two work-collection vectors are gone; the default page's vector stays.
    assert_eq!(
        vec_row_count(&conn),
        before - 2,
        "collection purge must drop exactly the purged collection's vec rows"
    );
    let default_pages: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE collection_id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(default_pages, 1);
    let orphans: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM page_embeddings_vec_384 v \
             WHERE NOT EXISTS (SELECT 1 FROM page_embeddings pe WHERE pe.vec_rowid = v.rowid)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(orphans, 0, "collection purge must not orphan vec rows");
}

#[test]
fn destroy_namespace_purging_all_pages_zeroes_the_vec_table() {
    let conn = open_test_db();
    insert_page(&conn, "notes/a", "notes", "q0001", 1, "first doomed page");
    insert_page(&conn, "notes/b", "notes", "q0001", 1, "second doomed page");
    run_with_batch(&conn, None, true, false, Some(8), false).unwrap();
    assert!(vec_row_count(&conn) >= 2);

    namespace::destroy_namespace(&conn, "q0001").unwrap();

    assert_eq!(
        vec_row_count(&conn),
        0,
        "purging every page in the namespace must empty the vec table"
    );
}
