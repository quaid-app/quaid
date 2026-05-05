use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quaid::commands::ingest;
use quaid::core::conversation::{
    extractor::{SlmClient, Worker},
    slm::SlmError,
    supersede::ResolvingFactWriter,
};
use quaid::core::db;
use quaid::mcp::server::{
    MemoryAddTurnInput, MemoryCloseSessionInput, MemorySearchInput, QuaidServer,
};
use rmcp::model::{CallToolResult, RawContent};
use rusqlite::Connection;

struct Harness {
    _dir: tempfile::TempDir,
    vault_root: PathBuf,
    server: QuaidServer,
    inspect: Connection,
}

impl Harness {
    fn new(window_turns: usize) -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("memory.db");
        let vault_root = dir.path().join("vault");
        fs::create_dir_all(&vault_root).unwrap();

        let server_conn = db::open(db_path.to_str().unwrap()).unwrap();
        server_conn
            .execute(
                "UPDATE collections
                 SET root_path = ?1,
                     writable = 1,
                     is_write_target = 1,
                     state = 'active',
                     needs_full_sync = 0
                 WHERE id = 1",
                [vault_root.display().to_string()],
            )
            .unwrap();
        server_conn
            .execute(
                "INSERT OR REPLACE INTO config(key, value) VALUES
                    ('extraction.enabled', 'true'),
                    ('extraction.window_turns', ?1)",
                [window_turns.to_string()],
            )
            .unwrap();

        let inspect = db::open(db_path.to_str().unwrap()).unwrap();
        Self {
            _dir: dir,
            vault_root,
            server: QuaidServer::new(server_conn),
            inspect,
        }
    }
}

#[derive(Debug, Clone)]
struct StubSlm {
    outputs: Arc<Mutex<VecDeque<Result<String, SlmError>>>>,
}

impl StubSlm {
    fn with_results(outputs: impl IntoIterator<Item = Result<&'static str, SlmError>>) -> Self {
        Self {
            outputs: Arc::new(Mutex::new(
                outputs
                    .into_iter()
                    .map(|result| result.map(str::to_string))
                    .collect(),
            )),
        }
    }
}

impl SlmClient for StubSlm {
    fn infer(&self, _alias: &str, _prompt: &str, _max_tokens: usize) -> Result<String, SlmError> {
        self.outputs
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok("{\"facts\":[]}".to_string()))
    }
}

fn extract_text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|content| match &content.raw {
            RawContent::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn markdown_files(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut entries = entries
            .flatten()
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, files);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
                files.push(path);
            }
        }
    }

    let mut files = Vec::new();
    walk(root, &mut files);
    files
}

#[test]
fn turn_capture_close_extract_fact_and_search_smoke() {
    let harness = Harness::new(50);

    for ordinal in 0..50 {
        let role = if ordinal % 2 == 0 {
            "user"
        } else {
            "assistant"
        };
        let content = if ordinal == 42 {
            "Project Cinder decision: keep the local-only extraction cache path."
        } else {
            "Project Cinder planning context."
        };
        let timestamp = format!("2026-05-05T09:{:02}:00Z", ordinal % 60);
        harness
            .server
            .memory_add_turn(MemoryAddTurnInput {
                session_id: "smoke-session".to_string(),
                role: role.to_string(),
                content: content.to_string(),
                timestamp: Some(timestamp),
                metadata: None,
                namespace: None,
            })
            .unwrap();
    }

    let close_result = harness
        .server
        .memory_close_session(MemoryCloseSessionInput {
            session_id: "smoke-session".to_string(),
            namespace: None,
        })
        .unwrap();
    let close_payload: serde_json::Value =
        serde_json::from_str(&extract_text(&close_result)).unwrap();
    assert_eq!(close_payload["extraction_triggered"], true);

    let slm = StubSlm::with_results([Ok(
        r#"{"facts":[{"kind":"decision","chose":"local-cache-path","summary":"The Project Cinder team decided to keep the extraction pipeline on a local-only cache path."}]}"#,
    )]);
    let worker = Worker::new(&harness.inspect, slm, ResolvingFactWriter)
        .unwrap()
        .with_limits(Duration::from_millis(1), 128);

    let processed = worker
        .process_next_job()
        .unwrap()
        .expect("queued close job");
    assert_eq!(processed.session_id, "smoke-session");

    let extracted_dir = harness.vault_root.join("extracted");
    let extracted_files = markdown_files(&extracted_dir);
    assert!(
        !extracted_files.is_empty(),
        "expected at least one extracted fact file under {}",
        extracted_dir.display()
    );
    for file in &extracted_files {
        ingest::run(&harness.inspect, file.to_str().unwrap(), false).unwrap();
    }

    let search = harness
        .server
        .memory_search(MemorySearchInput {
            query: "Project Cinder local-only cache".to_string(),
            collection: None,
            namespace: None,
            wing: None,
            limit: Some(5),
            include_superseded: Some(false),
        })
        .unwrap();
    let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&search)).unwrap();
    assert!(
        rows.iter().any(|row| {
            row["summary"]
                .as_str()
                .is_some_and(|summary| summary.contains("Project Cinder"))
        }),
        "expected extracted fact in search results: {rows:?}"
    );
}
