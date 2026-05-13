#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

#[path = "common/put_fixtures.rs"]
mod put_fixtures;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use put_fixtures::open_test_db as open_writable_db;
use quaid::commands::ingest;
use quaid::commands::put::put_from_string;
use quaid::core::db;
use quaid::core::migrate::export_dir;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

fn open_test_db(path: &Path) -> Connection {
    db::open(path.to_str().unwrap()).unwrap()
}

fn page_count(conn: &Connection) -> usize {
    conn.query_row("SELECT COUNT(*) FROM pages", [], |row| row.get::<_, i64>(0))
        .unwrap() as usize
}

fn collect_markdown_files(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "md") {
                files.push(path);
            }
        }
    }

    let mut files = Vec::new();
    walk(root, &mut files);
    files
}

fn ingest_markdown_tree(conn: &Connection, root: &Path) -> usize {
    let mut imported = 0usize;
    for file in collect_markdown_files(root) {
        let before = page_count(conn);
        ingest::run(conn, file.to_str().unwrap(), false).unwrap();
        let after = page_count(conn);
        imported += after.saturating_sub(before);
    }
    imported
}

fn exported_file_hashes(root: &Path) -> BTreeMap<String, String> {
    fn collect_hashes(root: &Path, dir: &Path, hashes: &mut BTreeMap<String, String>) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();

        for path in entries {
            if path.is_dir() {
                collect_hashes(root, &path, hashes);
            } else if path.extension().is_some_and(|ext| ext == "md") {
                let normalized = fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                let hash = format!("{:x}", Sha256::digest(normalized.as_bytes()));
                hashes.insert(relative, hash);
            }
        }
    }

    let mut hashes = BTreeMap::new();
    collect_hashes(root, root, &mut hashes);
    hashes
}

#[test]
fn import_export_reimport_preserves_page_count_and_rendered_content_hashes() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");

    let source_db_dir = tempfile::TempDir::new().unwrap();
    let source_conn = open_test_db(&source_db_dir.path().join("source.db"));
    let initial_imported = ingest_markdown_tree(&source_conn, &fixtures_dir);
    let original_page_count = page_count(&source_conn);

    assert_eq!(initial_imported, original_page_count);

    let first_export_root = tempfile::TempDir::new().unwrap();
    let exported_count = export_dir(&source_conn, first_export_root.path()).unwrap();
    assert_eq!(exported_count, original_page_count);
    let first_export_hashes = exported_file_hashes(first_export_root.path());

    let roundtrip_db_dir = tempfile::TempDir::new().unwrap();
    let roundtrip_conn = open_test_db(&roundtrip_db_dir.path().join("roundtrip.db"));
    ingest_markdown_tree(&roundtrip_conn, first_export_root.path());
    assert_eq!(page_count(&roundtrip_conn), original_page_count);

    let second_export_root = tempfile::TempDir::new().unwrap();
    let second_exported_count = export_dir(&roundtrip_conn, second_export_root.path()).unwrap();
    assert_eq!(second_exported_count, original_page_count);

    let second_export_hashes = exported_file_hashes(second_export_root.path());
    assert_eq!(second_export_hashes, first_export_hashes);
}

// ── Task 11.1: structured frontmatter + derived edge round-trip ──────────

fn seed_target(conn: &Connection, slug: &str) {
    put_from_string(
        conn,
        slug,
        "---\ntitle: target\ntype: concept\n---\nstub\n",
        None,
    )
    .unwrap();
}

fn derived_edge_set(conn: &Connection, from_slug: &str) -> BTreeMap<String, (String, String)> {
    let mut stmt = conn
        .prepare(
            "SELECT p_to.slug, l.relationship, l.source_kind \
             FROM links l \
             JOIN pages p_from ON p_from.id = l.from_page_id \
             JOIN pages p_to   ON p_to.id   = l.to_page_id \
             WHERE p_from.slug = ?1 AND l.source_kind IN ('frontmatter','wiki_link') \
             ORDER BY p_to.slug, l.relationship, l.source_kind",
        )
        .unwrap();
    let rows = stmt
        .query_map([from_slug], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap();
    let mut out = BTreeMap::new();
    for row in rows {
        let (to_slug, rel, kind) = row.unwrap();
        out.insert(format!("{to_slug}|{rel}|{kind}"), (rel, kind));
    }
    out
}

fn frontmatter_json(conn: &Connection, slug: &str) -> serde_json::Value {
    let raw: String = conn
        .query_row(
            "SELECT frontmatter FROM pages WHERE slug = ?1",
            [slug],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[test]
fn structured_frontmatter_and_derived_edges_survive_export_reimport() {
    // Round-trip a page whose YAML frontmatter holds arrays and an object
    // (`links:` list of objects, `tags:` list, `related:` list). After
    // export → re-ingest the structured frontmatter JSON must remain
    // equivalent and the derived edge set (frontmatter + wiki_link) must
    // match exactly.

    let source = open_writable_db();
    seed_target(&source, "companies/brex");
    seed_target(&source, "people/bob");
    seed_target(&source, "people/carol");

    let md = concat!(
        "---\n",
        "slug: people/alice\n",
        "title: Alice\n",
        "type: person\n",
        "tags: [founder, investor]\n",
        "links:\n",
        "  - target: companies/brex\n",
        "    type: founded\n",
        "    valid_from: 2017-01-01\n",
        "related:\n",
        "  - people/bob\n",
        "---\n",
        "# Alice\n\nAlice collaborates with [[people/carol]].\n",
    );
    put_from_string(&source, "people/alice", md, None).unwrap();

    let baseline_edges = derived_edge_set(&source, "people/alice");
    let baseline_fm = frontmatter_json(&source, "people/alice");
    assert!(
        baseline_fm
            .get("links")
            .map(|v| v.is_array())
            .unwrap_or(false),
        "baseline frontmatter must persist `links` as an array, got {baseline_fm}"
    );
    assert!(
        baseline_fm
            .get("tags")
            .map(|v| v.is_array())
            .unwrap_or(false),
        "baseline frontmatter must persist `tags` as an array, got {baseline_fm}"
    );
    assert!(
        baseline_edges
            .keys()
            .any(|k| k == "companies/brex|founded|frontmatter"),
        "expected baseline frontmatter edge to companies/brex: {baseline_edges:?}"
    );
    assert!(
        baseline_edges
            .keys()
            .any(|k| k == "people/carol|related|wiki_link"),
        "expected baseline wiki_link edge to people/carol: {baseline_edges:?}"
    );

    let export_root = tempfile::TempDir::new().unwrap();
    export_dir(&source, export_root.path()).unwrap();

    // Locate the exported alice page (path layout may vary by collection root).
    let alice_md = collect_markdown_files(export_root.path())
        .into_iter()
        .find(|p| p.file_name().and_then(|n| n.to_str()) == Some("alice.md"))
        .expect("exported alice.md not found under export root");

    let roundtrip = open_writable_db();
    // Re-seed targets so resolution succeeds in the fresh DB.
    seed_target(&roundtrip, "companies/brex");
    seed_target(&roundtrip, "people/bob");
    seed_target(&roundtrip, "people/carol");

    ingest::run(&roundtrip, alice_md.to_str().unwrap(), false).unwrap();

    let roundtrip_edges = derived_edge_set(&roundtrip, "people/alice");
    let roundtrip_fm = frontmatter_json(&roundtrip, "people/alice");

    assert_eq!(
        roundtrip_edges, baseline_edges,
        "derived edge set must be equivalent across export → re-import"
    );
    assert_eq!(
        roundtrip_fm.get("tags"),
        baseline_fm.get("tags"),
        "tags JSON must round-trip equivalently"
    );
    assert_eq!(
        roundtrip_fm.get("links"),
        baseline_fm.get("links"),
        "structured links JSON must round-trip equivalently"
    );
    assert_eq!(
        roundtrip_fm.get("related"),
        baseline_fm.get("related"),
        "related JSON must round-trip equivalently"
    );
}
