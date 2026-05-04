mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use std::path::Path;
use std::process::{Command, Output};

use quaid::core::db;
use quaid::core::progressive::progressive_retrieve;
use quaid::core::types::SearchResult;
use quaid::mcp::server::{
    MemoryGetInput, MemoryGraphInput, MemoryPutInput, MemoryQueryInput, MemorySearchInput,
    QuaidServer,
};
use rmcp::model::ErrorCode;
use rusqlite::Connection;
use serde_json::Value;

fn open_test_db(path: &Path) -> Connection {
    db::open(path.to_str().unwrap()).unwrap()
}

fn run_quaid(db_path: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    command
        .arg("--db")
        .arg(db_path)
        .args(args)
        .output()
        .expect("run quaid")
}

fn extract_text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|content| match &content.raw {
            rmcp::model::RawContent::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn put_page(server: &QuaidServer, slug: &str, content: &str, expected_version: Option<i64>) {
    server
        .memory_put(MemoryPutInput {
            slug: slug.to_string(),
            content: content.to_string(),
            expected_version,
            namespace: None,
        })
        .unwrap();
}

fn search_result(slug: &str) -> SearchResult {
    SearchResult {
        slug: slug.to_string(),
        title: slug.to_string(),
        summary: slug.to_string(),
        score: 1.0,
        wing: "facts".to_string(),
    }
}

fn slugs(rows: &[Value]) -> Vec<String> {
    let mut slugs = rows
        .iter()
        .map(|row| row["slug"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    slugs.sort();
    slugs
}

#[test]
fn supersede_chain_write_rejects_non_head_and_memory_get_returns_successor_pointer() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let server = QuaidServer::new(open_test_db(&db_path));

    put_page(
        &server,
        "facts/a",
        "---\ntitle: Fact A\ntype: fact\n---\nshared supersede marker\n",
        None,
    );
    put_page(
        &server,
        "facts/b",
        "---\ntitle: Fact B\ntype: fact\nsupersedes: facts/a\n---\nshared supersede marker\n",
        None,
    );
    put_page(
        &server,
        "facts/c",
        "---\ntitle: Fact C\ntype: fact\nsupersedes: facts/b\n---\nshared supersede marker\n",
        None,
    );

    let error = server
        .memory_put(MemoryPutInput {
            slug: "facts/rejected".to_string(),
            content: "---\ntitle: Rejected\ntype: fact\nsupersedes: facts/a\n---\nshared supersede marker\n"
                .to_string(),
            expected_version: None,
            namespace: None,
        })
        .unwrap_err();
    assert_eq!(error.code, ErrorCode(-32009));
    assert!(error.message.contains("SupersedeConflictError"));

    let db = open_test_db(&db_path);
    let a_id: i64 = db
        .query_row("SELECT id FROM pages WHERE slug = 'facts/a'", [], |row| {
            row.get(0)
        })
        .unwrap();
    let b_id: i64 = db
        .query_row("SELECT id FROM pages WHERE slug = 'facts/b'", [], |row| {
            row.get(0)
        })
        .unwrap();
    let c_id: i64 = db
        .query_row("SELECT id FROM pages WHERE slug = 'facts/c'", [], |row| {
            row.get(0)
        })
        .unwrap();
    let links: Vec<(String, Option<i64>)> = db
        .prepare("SELECT slug, superseded_by FROM pages WHERE slug LIKE 'facts/%' ORDER BY slug")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    drop(db);

    assert_eq!(
        links,
        vec![
            ("facts/a".to_string(), Some(b_id)),
            ("facts/b".to_string(), Some(c_id)),
            ("facts/c".to_string(), None),
        ]
    );

    let superseded_a: Value = serde_json::from_str(&extract_text(
        &server
            .memory_get(MemoryGetInput {
                slug: "facts/a".to_string(),
            })
            .unwrap(),
    ))
    .unwrap();
    let head_c: Value = serde_json::from_str(&extract_text(
        &server
            .memory_get(MemoryGetInput {
                slug: "facts/c".to_string(),
            })
            .unwrap(),
    ))
    .unwrap();

    assert_eq!(superseded_a["slug"], "default::facts/a");
    assert_eq!(superseded_a["superseded_by"], "default::facts/b");
    assert!(superseded_a["supersedes"].is_null());
    assert_eq!(head_c["supersedes"], "default::facts/b");
    assert!(head_c["superseded_by"].is_null());
    assert!(a_id > 0);
}

#[test]
fn retrieval_defaults_to_heads_and_include_superseded_restores_history() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let server = QuaidServer::new(open_test_db(&db_path));

    put_page(
        &server,
        "facts/a",
        "---\ntitle: Fact A\ntype: fact\n---\nshared retrieval marker\n",
        None,
    );
    put_page(
        &server,
        "facts/b",
        "---\ntitle: Fact B\ntype: fact\nsupersedes: facts/a\n---\nshared retrieval marker\n",
        None,
    );
    put_page(
        &server,
        "facts/c",
        "---\ntitle: Fact C\ntype: fact\nsupersedes: facts/b\n---\nshared retrieval marker\n",
        None,
    );

    let default_search: Vec<Value> = serde_json::from_str(&extract_text(
        &server
            .memory_search(MemorySearchInput {
                query: "shared retrieval marker".to_string(),
                collection: None,
                namespace: None,
                wing: None,
                limit: None,
                include_superseded: None,
            })
            .unwrap(),
    ))
    .unwrap();
    let historical_search: Vec<Value> = serde_json::from_str(&extract_text(
        &server
            .memory_search(MemorySearchInput {
                query: "shared retrieval marker".to_string(),
                collection: None,
                namespace: None,
                wing: None,
                limit: None,
                include_superseded: Some(true),
            })
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(slugs(&default_search), vec!["default::facts/c".to_string()]);
    assert_eq!(
        slugs(&historical_search),
        vec![
            "default::facts/a".to_string(),
            "default::facts/b".to_string(),
            "default::facts/c".to_string(),
        ]
    );

    let default_query: Vec<Value> = serde_json::from_str(&extract_text(
        &server
            .memory_query(MemoryQueryInput {
                query: "shared retrieval marker".to_string(),
                collection: None,
                namespace: None,
                wing: None,
                limit: Some(10),
                depth: None,
                include_superseded: None,
            })
            .unwrap(),
    ))
    .unwrap();
    let historical_query: Vec<Value> = serde_json::from_str(&extract_text(
        &server
            .memory_query(MemoryQueryInput {
                query: "shared retrieval marker".to_string(),
                collection: None,
                namespace: None,
                wing: None,
                limit: Some(10),
                depth: None,
                include_superseded: Some(true),
            })
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(slugs(&default_query), vec!["default::facts/c".to_string()]);
    assert_eq!(
        slugs(&historical_query),
        vec![
            "default::facts/a".to_string(),
            "default::facts/b".to_string(),
            "default::facts/c".to_string(),
        ]
    );

    let db = open_test_db(&db_path);
    let progressive_default = progressive_retrieve(
        vec![
            search_result("default::facts/a"),
            search_result("default::facts/b"),
            search_result("default::facts/c"),
        ],
        10_000,
        1,
        None,
        false,
        &db,
    )
    .unwrap();
    let progressive_history = progressive_retrieve(
        vec![
            search_result("default::facts/a"),
            search_result("default::facts/b"),
            search_result("default::facts/c"),
        ],
        10_000,
        1,
        None,
        true,
        &db,
    )
    .unwrap();
    assert_eq!(
        progressive_default
            .iter()
            .map(|row| row.slug.clone())
            .collect::<Vec<_>>(),
        vec!["default::facts/c".to_string()]
    );
    assert_eq!(
        progressive_history
            .iter()
            .map(|row| row.slug.clone())
            .collect::<Vec<_>>(),
        vec![
            "default::facts/a".to_string(),
            "default::facts/b".to_string(),
            "default::facts/c".to_string(),
        ]
    );

    let graph: Value = serde_json::from_str(&extract_text(
        &server
            .memory_graph(MemoryGraphInput {
                slug: "facts/c".to_string(),
                depth: Some(2),
                temporal: Some("all".to_string()),
            })
            .unwrap(),
    ))
    .unwrap();
    let edge_pairs = graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|edge| edge["relationship"] == "superseded_by")
        .map(|edge| {
            (
                edge["from"].as_str().unwrap().to_string(),
                edge["to"].as_str().unwrap().to_string(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        edge_pairs,
        vec![
            (
                "default::facts/b".to_string(),
                "default::facts/c".to_string()
            ),
            (
                "default::facts/a".to_string(),
                "default::facts/b".to_string()
            ),
        ]
    );

    drop(db);
    drop(server);

    let cli_search_default = run_quaid(&db_path, &["--json", "search", "shared retrieval marker"]);
    assert!(cli_search_default.status.success());
    let cli_search_default_rows: Vec<Value> =
        serde_json::from_slice(&cli_search_default.stdout).unwrap();
    assert_eq!(
        slugs(&cli_search_default_rows),
        vec!["default::facts/c".to_string()]
    );

    let cli_search_history = run_quaid(
        &db_path,
        &[
            "--json",
            "search",
            "shared retrieval marker",
            "--include-superseded",
        ],
    );
    assert!(cli_search_history.status.success());
    let cli_search_history_rows: Vec<Value> =
        serde_json::from_slice(&cli_search_history.stdout).unwrap();
    assert_eq!(
        slugs(&cli_search_history_rows),
        vec![
            "default::facts/a".to_string(),
            "default::facts/b".to_string(),
            "default::facts/c".to_string(),
        ]
    );

    let cli_query_default = run_quaid(&db_path, &["--json", "query", "shared retrieval marker"]);
    assert!(cli_query_default.status.success());
    let cli_query_default_rows: Vec<Value> =
        serde_json::from_slice(&cli_query_default.stdout).unwrap();
    assert_eq!(
        slugs(&cli_query_default_rows),
        vec!["default::facts/c".to_string()]
    );

    let cli_query_history = run_quaid(
        &db_path,
        &[
            "--json",
            "query",
            "shared retrieval marker",
            "--include-superseded",
        ],
    );
    assert!(cli_query_history.status.success());
    let cli_query_history_rows: Vec<Value> =
        serde_json::from_slice(&cli_query_history.stdout).unwrap();
    assert_eq!(
        slugs(&cli_query_history_rows),
        vec![
            "default::facts/a".to_string(),
            "default::facts/b".to_string(),
            "default::facts/c".to_string(),
        ]
    );
}
