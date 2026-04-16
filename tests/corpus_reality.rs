//! Corpus-reality integration tests — validates end-to-end brain behaviour
//! against the fixture corpus.
//!
//! Scenarios tested:
//!   1. Import completeness    — all fixture files produce pages
//!   2. SMS retrieval          — exact slug → top-1 result
//!   3. Timeline retrieval     — known fact text → correct page in top-5
//!   4. Duplicate ingest       — no duplicate pages after re-import
//!   5. Conflicting ingest     — contradiction detected in check
//!   6. Idempotent round-trip  — export → reimport → export → semantic diff = 0
//!   7. Latency gate           — p95 < 250ms over 100 queries (release build, #[ignore])

use std::fs;
use std::path::Path;
use std::time::Instant;

use gbrain::commands::embed;
use gbrain::core::assertions;
use gbrain::core::db;
use gbrain::core::fts::search_fts;
use gbrain::core::migrate::{export_dir, import_dir};
use gbrain::core::search::hybrid_search;

fn open_test_db() -> rusqlite::Connection {
    db::open(":memory:").expect("open in-memory DB")
}

fn open_disk_db() -> (rusqlite::Connection, tempfile::TempDir) {
    let dir = tempfile::TempDir::new().expect("create temp dir");
    let db_path = dir.path().join("brain.db");
    let conn = db::open(db_path.to_str().unwrap()).expect("open DB");
    (conn, dir)
}

fn fixtures_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn count_fixture_files() -> usize {
    fs::read_dir(fixtures_dir())
        .expect("read fixtures dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
        .count()
}

// ── 1. Import completeness ────────────────────────────────────────────────────

#[test]
fn import_completeness_all_fixtures_produce_pages() {
    let conn = open_test_db();
    let fixture_count = count_fixture_files();

    let stats = import_dir(&conn, &fixtures_dir(), false).expect("import fixtures");

    assert_eq!(
        stats.imported, fixture_count,
        "expected {fixture_count} pages imported, got {}",
        stats.imported
    );
    assert_eq!(
        stats.skipped, 0,
        "no files should be skipped on first import"
    );

    let page_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
        .expect("count pages");
    assert_eq!(
        page_count, fixture_count as i64,
        "DB should contain exactly {fixture_count} pages"
    );
}

// ── 2. SMS (exact-slug) retrieval ─────────────────────────────────────────────

#[test]
fn sms_exact_slug_returns_page_as_top_1() {
    let conn = open_test_db();
    import_dir(&conn, &fixtures_dir(), false).expect("import fixtures");
    embed::run(&conn, None, true, false).expect("embed pages");

    // All fixture slugs should be retrievable via exact-slug lookup
    let slugs = [
        "people/pedro-franceschi",
        "people/henrique-dubugras",
        "companies/brex",
        "companies/acme",
        "projects/gigabrain",
    ];

    for slug in &slugs {
        let results = hybrid_search(slug, None, &conn, 5).expect("search");
        assert!(
            !results.is_empty(),
            "exact-slug search for '{slug}' returned no results"
        );
        assert_eq!(
            results[0].slug, *slug,
            "top-1 for '{slug}' should be the page itself"
        );
    }
}

// ── 3. Timeline retrieval ─────────────────────────────────────────────────────

#[test]
fn timeline_retrieval_known_fact_appears_in_top_5() {
    let conn = open_test_db();
    import_dir(&conn, &fixtures_dir(), false).expect("import fixtures");
    embed::run(&conn, None, true, false).expect("embed pages");

    // Known facts from fixture timelines — should surface correct pages
    let cases = [
        ("Henrique Dubugras Brex founder", "people/henrique-dubugras"),
        (
            "corporate card financial infrastructure startups",
            "companies/brex",
        ),
        ("knowledge brain SQLite embeddings", "projects/gigabrain"),
    ];

    for (query, expected_slug) in &cases {
        let results = hybrid_search(query, None, &conn, 5).expect("hybrid search");
        let slugs: Vec<&str> = results.iter().map(|r| r.slug.as_str()).collect();
        assert!(
            slugs.contains(expected_slug),
            "query '{query}' should return '{expected_slug}' in top-5, got: {slugs:?}"
        );
    }
}

// ── 4. Duplicate ingest ───────────────────────────────────────────────────────

#[test]
fn duplicate_ingest_produces_no_additional_pages() {
    let conn = open_test_db();

    // First import
    let stats1 = import_dir(&conn, &fixtures_dir(), false).expect("first import");
    let page_count_after_first: i64 = conn
        .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
        .expect("count pages");

    // Second import — should skip all (hash-based idempotency)
    let stats2 = import_dir(&conn, &fixtures_dir(), false).expect("second import");
    let page_count_after_second: i64 = conn
        .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
        .expect("count pages");

    assert_eq!(
        stats2.imported, 0,
        "second import should import 0 files (all cached)"
    );
    assert_eq!(
        stats2.skipped, stats1.imported,
        "all files from first import should be skipped"
    );
    assert_eq!(
        page_count_after_first, page_count_after_second,
        "page count should not change on second import"
    );
}

// ── 5. Conflicting ingest → contradiction detected ────────────────────────────

#[test]
fn conflicting_ingest_contradiction_is_detected() {
    let conn = open_test_db();

    // Insert two pages with conflicting facts about the same entity
    conn.execute(
        "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES ('people/alice', 'person', 'Alice', '', \
                 'Alice works at Acme Corp.', \
                 '', '{}', 'people', '', 1)",
        [],
    )
    .expect("insert page 1");

    conn.execute(
        "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES ('sources/update', 'concept', 'Update', '', \
                 'Alice works at Beta Corp.', \
                 '', '{}', 'sources', '', 1)",
        [],
    )
    .expect("insert page 2");

    // Extract assertions from both pages
    let page_a = gbrain::commands::get::get_page(&conn, "people/alice").expect("get alice");
    let page_b = gbrain::commands::get::get_page(&conn, "sources/update").expect("get update");
    assertions::extract_assertions(&page_a, &conn).expect("extract assertions alice");
    assertions::extract_assertions(&page_b, &conn).expect("extract assertions update");

    // Run check against alice — should detect contradiction
    let contradictions =
        assertions::check_assertions("people/alice", &conn).expect("check assertions");

    assert!(
        !contradictions.is_empty(),
        "contradiction between Acme Corp and Beta Corp should be detected"
    );

    let all_desc: String = contradictions
        .iter()
        .map(|c| c.description.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    assert!(
        all_desc.contains("Acme Corp") || all_desc.contains("Beta Corp"),
        "contradiction description should name the conflicting companies: {all_desc}"
    );
}

// ── 6. Idempotent round-trip ──────────────────────────────────────────────────

#[test]
fn idempotent_roundtrip_export_reimport_export_is_zero_diff() {
    let (conn, _dir) = open_disk_db();

    import_dir(&conn, &fixtures_dir(), false).expect("import fixtures");

    let export_dir1 = tempfile::TempDir::new().expect("export dir 1");
    let count1 = export_dir(&conn, export_dir1.path()).expect("export 1");
    assert_eq!(
        count1,
        count_fixture_files(),
        "export 1 should produce same count as fixtures"
    );

    // Re-import into a fresh DB
    let (conn2, _dir2) = open_disk_db();
    let stats = import_dir(&conn2, export_dir1.path(), false).expect("reimport");
    assert_eq!(
        stats.imported, count1,
        "reimport should import all exported files"
    );

    // Export again
    let export_dir2 = tempfile::TempDir::new().expect("export dir 2");
    let count2 = export_dir(&conn2, export_dir2.path()).expect("export 2");
    assert_eq!(
        count1, count2,
        "both exports should have the same page count"
    );

    // Compare page content semantically (slug, truth, timeline)
    let mut pages1 = collect_exported_pages(export_dir1.path());
    let mut pages2 = collect_exported_pages(export_dir2.path());
    pages1.sort_by(|a, b| a.0.cmp(&b.0));
    pages2.sort_by(|a, b| a.0.cmp(&b.0));

    assert_eq!(
        pages1.len(),
        pages2.len(),
        "page counts should match between exports"
    );

    for (i, ((slug1, content1), (slug2, content2))) in pages1.iter().zip(pages2.iter()).enumerate()
    {
        assert_eq!(slug1, slug2, "slug mismatch at index {i}");
        // Normalize line endings before comparison: fixtures may use CRLF,
        // but the semantic content must be identical after round-trip.
        let norm1 = content1.replace("\r\n", "\n");
        let norm2 = content2.replace("\r\n", "\n");
        assert_eq!(
            norm1, norm2,
            "content mismatch for {slug1}: round-trip should be lossless (normalized LF)"
        );
    }
}

/// Walk an export directory and collect (relative_path, content) pairs.
fn collect_exported_pages(root: &Path) -> Vec<(String, String)> {
    let mut pages = Vec::new();
    collect_recursive(root, root, &mut pages);
    pages
}

fn collect_recursive(root: &Path, dir: &Path, pages: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).expect("read dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_recursive(root, &path, pages);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let content = fs::read_to_string(&path).expect("read file");
            pages.push((rel, content));
        }
    }
}

// ── 7. Latency gate — p95 < 250ms (release build only) ───────────────────────

#[test]
#[ignore = "latency gate requires release build — run: cargo test --release --test corpus_reality latency -- --ignored"]
fn latency_100_queries_p95_under_250ms() {
    let conn = open_test_db();
    import_dir(&conn, &fixtures_dir(), false).expect("import fixtures");
    embed::run(&conn, None, true, false).expect("embed pages");

    let queries = [
        "who founded brex",
        "technology company developer tools",
        "knowledge brain sqlite embeddings",
        "corporate card fintech startup",
        "brazilian entrepreneur yc",
        "rust sqlite vector search",
        "developer productivity apis",
        "brex cto technical leadership",
        "co-founded enterprise software startup",
        "personal knowledge management",
    ];

    let mut durations_ms: Vec<f64> = Vec::with_capacity(100);

    for i in 0..100 {
        let query = queries[i % queries.len()];
        let start = Instant::now();
        let _ = hybrid_search(query, None, &conn, 10).expect("search");
        durations_ms.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    durations_ms.sort_by(|a, b| a.total_cmp(b));

    let p50 = percentile(&durations_ms, 50);
    let p95 = percentile(&durations_ms, 95);
    let p99 = percentile(&durations_ms, 99);

    eprintln!("Latency over 100 queries:");
    eprintln!("  p50: {p50:.1}ms");
    eprintln!("  p95: {p95:.1}ms");
    eprintln!("  p99: {p99:.1}ms");

    assert!(
        p95 < 250.0,
        "p95 latency {p95:.1}ms exceeds 250ms gate — run on release build for accurate measurement"
    );
}

fn percentile(sorted: &[f64], pct: usize) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = (sorted.len() * pct / 100).min(sorted.len() - 1);
    sorted[idx]
}

// ── FTS5 search coverage: fixture terms ──────────────────────────────────────

#[test]
fn fts5_search_finds_all_fixture_pages_by_distinctive_terms() {
    let conn = open_test_db();
    import_dir(&conn, &fixtures_dir(), false).expect("import fixtures");

    // Term → expected slug
    let cases: &[(&str, &str)] = &[
        ("Franceschi", "people/pedro-franceschi"),
        ("Dubugras", "people/henrique-dubugras"),
        ("Brex", "companies/brex"),
        ("GigaBrain", "projects/gigabrain"),
    ];

    for (term, expected_slug) in cases {
        let results = search_fts(term, None, &conn, 10).expect("fts search");
        let slugs: Vec<&str> = results.iter().map(|r| r.slug.as_str()).collect();
        assert!(
            slugs.contains(expected_slug),
            "FTS5 search for '{term}' should include '{expected_slug}', got: {slugs:?}"
        );
    }
}
