use std::fs;
use std::path::{Path, PathBuf};

use quaid::commands::ingest;
use quaid::core::conversation::supersede::{
    write_fact_in_context, FactWriteContext, Resolution,
};
use quaid::core::db;
use quaid::core::types::{ActionItemState, PreferenceStrength, RawFact};
use rusqlite::{params, Connection};

fn open_test_db(path: &Path) -> Connection {
    let conn = db::open(path.to_str().unwrap()).unwrap();
    conn.execute(
        "UPDATE collections
         SET root_path = ?1,
             state = 'active'
         WHERE id = 1",
        [path.parent().unwrap().display().to_string()],
    )
    .unwrap();
    conn
}

fn write_context(root: &Path, namespace: Option<&str>) -> FactWriteContext {
    FactWriteContext {
        collection_id: 1,
        root_path: root.to_path_buf(),
        namespace: namespace.unwrap_or_default().to_string(),
        session_id: "session-1".to_string(),
        source_turns: vec!["1".to_string(), "2".to_string()],
        extracted_at: "2026-05-05T09:00:00Z".to_string(),
        extracted_by: "phi-3.5-mini".to_string(),
    }
}

fn insert_existing_head(conn: &Connection, slug: &str) -> i64 {
    let frontmatter = serde_json::json!({
        "kind": "preference",
        "about": "programming-language"
    })
    .to_string();
    conn.execute(
        "INSERT INTO pages
             (collection_id, namespace, slug, uuid, type, title, summary, compiled_truth, timeline,
              frontmatter, wing, room, superseded_by, version)
         VALUES
             (1, '', ?1, ?2, 'preference', 'Programming language', 'Prefers Rust', 'Prefers Rust', '',
              ?3, '', '', NULL, 1)",
        params![slug, format!("uuid-{slug}"), frontmatter],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn preference_fact(summary: &str) -> RawFact {
    RawFact::Preference {
        about: "programming-language".to_string(),
        strength: Some(PreferenceStrength::High),
        summary: summary.to_string(),
    }
}

#[test]
fn write_fact_drop_resolution_writes_nothing_and_skips_db_mutation() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    let context = write_context(dir.path(), None);

    let result = write_fact_in_context(
        &Resolution::Drop {
            matched_slug: "existing".to_string(),
            cosine: 0.97,
        },
        &preference_fact("Matt prefers Rust"),
        &conn,
        &context,
    )
    .unwrap();

    assert!(result.slug.is_none());
    assert!(!dir.path().join("extracted").exists());
    let page_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(page_count, 0);
}

#[test]
fn write_fact_supersede_writes_markdown_then_ingest_updates_chain() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_test_db(&db_path);
    let prior_head_id = insert_existing_head(&conn, "existing-rust");
    let context = write_context(dir.path(), None);

    let result = write_fact_in_context(
        &Resolution::Supersede {
            prior_slug: "existing-rust".to_string(),
            cosine: 0.55,
        },
        &preference_fact("Matt switched to Zig"),
        &conn,
        &context,
    )
    .unwrap();

    let relative_path = result.relative_path.clone().unwrap();
    let full_path = dir
        .path()
        .join(relative_path.replace('/', std::path::MAIN_SEPARATOR_STR));
    let markdown = fs::read_to_string(&full_path).unwrap();
    assert!(relative_path.starts_with("extracted/preferences/"));
    assert!(markdown.contains("supersedes: existing-rust"));
    assert!(markdown.contains("corrected_via: null"));
    assert!(markdown.contains("source_turns: '[\"session-1:1\",\"session-1:2\"]'"));

    let before_ingest_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(before_ingest_count, 1);

    ingest::run(&conn, full_path.to_str().unwrap(), false).unwrap();

    let new_page_id: i64 = conn
        .query_row(
            "SELECT id FROM pages WHERE slug = ?1",
            [result.slug.as_deref().unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    let successor: Option<i64> = conn
        .query_row(
            "SELECT superseded_by FROM pages WHERE id = ?1",
            [prior_head_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(successor, Some(new_page_id));
}

#[test]
fn write_fact_coexist_writes_null_supersedes_without_page_insert() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    let context = write_context(dir.path(), None);

    let result = write_fact_in_context(
        &Resolution::Coexist,
        &preference_fact("Matt prefers Rust"),
        &conn,
        &context,
    )
    .unwrap();

    let full_path = dir
        .path()
        .join(result.relative_path.unwrap().replace('/', std::path::MAIN_SEPARATOR_STR));
    let markdown = fs::read_to_string(full_path).unwrap();
    assert!(markdown.contains("supersedes: null"));

    let page_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(page_count, 0);
}

#[test]
fn write_fact_slug_collision_appends_counter_suffix() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    let context = write_context(dir.path(), None);
    let fact = preference_fact("Matt prefers Rust");

    let first = write_fact_in_context(&Resolution::Coexist, &fact, &conn, &context).unwrap();
    let second = write_fact_in_context(&Resolution::Coexist, &fact, &conn, &context).unwrap();

    assert_ne!(first.slug, second.slug);
    assert!(second.slug.unwrap().ends_with("-2"));
}

#[test]
fn write_fact_respects_namespace_nested_output_path() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    let context = write_context(dir.path(), Some("alpha"));
    let fact = RawFact::ActionItem {
        who: Some("Fry".to_string()),
        what: "ship the worker".to_string(),
        status: ActionItemState::Open,
        due: None,
        summary: "Fry will ship the worker".to_string(),
    };

    let result = write_fact_in_context(&Resolution::Coexist, &fact, &conn, &context).unwrap();

    let relative_path = result.relative_path.unwrap();
    assert!(relative_path.starts_with("alpha/extracted/action-items/"));
    assert!(
        PathBuf::from(relative_path.replace('/', std::path::MAIN_SEPARATOR_STR))
            .components()
            .any(|component| component.as_os_str() == "alpha")
    );
}
