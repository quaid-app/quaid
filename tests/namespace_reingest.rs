#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Watcher-reingest and open-time backfill coverage for namespace-aware page
//! identity (issue #212): files under `<ns>/extracted/...` must land in
//! namespace `<ns>`, never overwrite same-slug pages in other namespaces,
//! and become visible to the supersede head partition
//! (`head_candidates` filters `namespace = ?2`).

#[path = "common/reconciler_fixtures.rs"]
mod common_reconciler_fixtures;

use common_reconciler_fixtures::*;
use quaid::core::conversation::supersede::{
    resolve_in_scope_with_similarity, write_fact_in_context, FactWriteContext, Resolution,
};
use quaid::core::file_state::{self, FileStat};
use quaid::core::reconciler::reconcile;
use quaid::core::types::RawFact;
use rusqlite::{params, Connection, OptionalExtension};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn page_row(conn: &Connection, namespace: &str, slug: &str) -> Option<(i64, String, i64)> {
    conn.query_row(
        "SELECT id, compiled_truth, version FROM pages WHERE namespace = ?1 AND slug = ?2",
        params![namespace, slug],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .optional()
    .unwrap()
}

fn preference_fact(summary: &str) -> RawFact {
    RawFact::Preference {
        about: "programming-language".to_string(),
        strength: None,
        summary: summary.to_string(),
    }
}

fn write_context(collection_id: i64, root: &Path, namespace: &str) -> FactWriteContext {
    FactWriteContext {
        collection_id,
        root_path: root.to_path_buf(),
        namespace: namespace.to_string(),
        session_id: "session-1".to_string(),
        source_turns: vec!["1".to_string()],
        extracted_at: "2026-06-01T09:00:00Z".to_string(),
        extracted_by: "phi-3.5-mini".to_string(),
    }
}

#[cfg(unix)]
#[test]
fn reingest_assigns_namespace_from_extracted_path_and_isolates_other_namespaces() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());

    // Established post-fix state: a same-slug fact already lives in ns-b.
    let ns_b_relative = "ns-b/extracted/preferences/lang-pref.md";
    let ns_b_body = "---\nslug: lang-pref\ntitle: lang-pref\ntype: preference\nkind: preference\nabout: programming-language\n---\nPrefers Go.\n";
    fs::create_dir_all(root.path().join("ns-b/extracted/preferences")).unwrap();
    fs::write(root.path().join(ns_b_relative), ns_b_body).unwrap();
    let ns_b_stat = stat_for(root.path(), ns_b_relative);
    let ns_b_sha = file_state::hash_file(&root.path().join(ns_b_relative)).unwrap();
    let ns_b_page_id = seed_page_with_identity(
        &conn,
        collection.id,
        SeededPageIdentity {
            slug: "lang-pref",
            uuid: "01969f11-9448-7d79-8d3f-c68f54761234",
            relative_path: ns_b_relative,
            stat: &ns_b_stat,
            sha256: &ns_b_sha,
            compiled_truth: "Prefers Go.",
            timeline: "",
        },
    );
    conn.execute(
        "UPDATE pages SET namespace = 'ns-b', type = 'preference' WHERE id = ?1",
        [ns_b_page_id],
    )
    .unwrap();

    // The watcher discovers a brand-new fact file in ns-a with the SAME slug.
    let ns_a_relative = "ns-a/extracted/preferences/lang-pref.md";
    let ns_a_body = "---\nslug: lang-pref\ntitle: lang-pref\ntype: preference\nkind: preference\nabout: programming-language\n---\nPrefers Rust.\n";
    fs::create_dir_all(root.path().join("ns-a/extracted/preferences")).unwrap();
    fs::write(root.path().join(ns_a_relative), ns_a_body).unwrap();

    reconcile(&conn, &collection).unwrap();

    let (_, ns_a_truth, _) =
        page_row(&conn, "ns-a", "lang-pref").expect("ns-a fact page must be created");
    assert!(
        ns_a_truth.contains("Prefers Rust."),
        "ns-a page must carry the ns-a file body, got: {ns_a_truth}"
    );

    let (ns_b_id_after, ns_b_truth, ns_b_version) =
        page_row(&conn, "ns-b", "lang-pref").expect("ns-b page must survive");
    assert_eq!(ns_b_id_after, ns_b_page_id, "ns-b page row must be intact");
    assert_eq!(
        ns_b_truth, "Prefers Go.",
        "ns-b page content must not be overwritten by the ns-a file"
    );
    assert_eq!(ns_b_version, 1, "ns-b page must not be version-bumped");

    assert_eq!(
        page_row(&conn, "", "lang-pref"),
        None,
        "no global-namespace row may be fabricated for a namespaced file"
    );
}

#[cfg(unix)]
#[test]
fn head_candidates_find_prior_head_after_watcher_ingest_of_namespaced_fact() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();
    let collection = insert_collection(&conn, root.path());
    let context = write_context(collection.id, root.path(), "ns-a");
    let similarity = |_: &str, _: &str| Ok(0.5_f64);

    // First fact: empty head partition -> Coexist, file written to
    // ns-a/extracted/... and ingested by the watcher.
    let first = preference_fact("Prefers Rust for systems work.");
    let resolution =
        resolve_in_scope_with_similarity(&first, &conn, collection.id, "ns-a", similarity).unwrap();
    assert!(matches!(resolution, Resolution::Coexist));
    let written = write_fact_in_context(&resolution, &first, &conn, &context).unwrap();
    let first_slug = written.slug.expect("coexist write allocates a slug");
    let relative_path = written.relative_path.expect("coexist write emits a file");
    assert!(
        relative_path.starts_with("ns-a/extracted/"),
        "fact file must live under the namespace prefix, got {relative_path}"
    );

    reconcile(&conn, &collection).unwrap();

    let (_, _, _) = page_row(&conn, "ns-a", &first_slug)
        .expect("watcher-ingested fact page must land in namespace ns-a (issue #212)");

    // Second fact in the same (kind, key) partition: the prior head MUST be
    // found. Pre-fix, the page row sat in '' so head_candidates (namespace =
    // ?2) saw nothing and resolution stayed Coexist forever, accumulating
    // duplicates.
    let second = preference_fact("Prefers Zig now.");
    let resolution =
        resolve_in_scope_with_similarity(&second, &conn, collection.id, "ns-a", similarity)
            .unwrap();
    match resolution {
        Resolution::Supersede { prior_slug, .. } => assert_eq!(prior_slug, first_slug),
        other => panic!("expected Supersede of the watcher-ingested head, got {other:?}"),
    }

    // Other namespaces stay isolated: their partitions are still empty.
    let elsewhere =
        resolve_in_scope_with_similarity(&second, &conn, collection.id, "ns-other", similarity)
            .unwrap();
    assert!(matches!(elsewhere, Resolution::Coexist));
}

fn seed_pre_fix_page(
    conn: &Connection,
    namespace: &str,
    slug: &str,
    relative_path: Option<&str>,
) -> i64 {
    conn.execute(
        "INSERT INTO pages (collection_id, namespace, slug, uuid, type, title, compiled_truth)
         VALUES (1, ?1, ?2, ?3, 'preference', ?2, 'body')",
        params![namespace, slug, format!("uuid-{namespace}-{slug}")],
    )
    .unwrap();
    let page_id = conn.last_insert_rowid();
    if let Some(relative_path) = relative_path {
        let stat = FileStat {
            mtime_ns: 1,
            ctime_ns: Some(1),
            size_bytes: 4,
            inode: Some(page_id),
        };
        file_state::upsert_file_state(conn, 1, relative_path, page_id, &stat, "deadbeef").unwrap();
    }
    page_id
}

#[test]
fn open_time_backfill_rehomes_pre_fix_rows_from_file_state_paths() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = quaid::core::db::open(db_path.to_str().unwrap()).unwrap();

    // Pre-fix shape: watcher-ingested namespaced facts sat in namespace ''.
    let rehomed = seed_pre_fix_page(
        &conn,
        "",
        "pref-a",
        Some("ns-a/extracted/preferences/pref-a.md"),
    );
    // Conflict shape: the ns-b target slot is already occupied.
    let conflicted = seed_pre_fix_page(
        &conn,
        "",
        "pref-b",
        Some("ns-b/extracted/preferences/pref-b.md"),
    );
    let occupant = seed_pre_fix_page(&conn, "ns-b", "pref-b", None);
    // Plain vault page: no namespace prefix, must stay global.
    let global = seed_pre_fix_page(&conn, "", "global-note", Some("notes/global-note.md"));
    drop(conn);

    let conn = quaid::core::db::open(db_path.to_str().unwrap()).unwrap();
    let namespace_of = |page_id: i64| -> String {
        conn.query_row(
            "SELECT namespace FROM pages WHERE id = ?1",
            [page_id],
            |row| row.get(0),
        )
        .unwrap()
    };

    assert_eq!(
        namespace_of(rehomed),
        "ns-a",
        "pre-fix row with a namespaced file_state path must be re-homed"
    );
    assert_eq!(
        namespace_of(conflicted),
        "",
        "rows whose target slot is occupied are skipped, not clobbered"
    );
    assert_eq!(namespace_of(occupant), "ns-b");
    assert_eq!(namespace_of(global), "");

    // Idempotent: a third open changes nothing.
    drop(conn);
    let conn = quaid::core::db::open(db_path.to_str().unwrap()).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pages WHERE namespace = 'ns-a'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}
