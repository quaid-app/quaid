mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quaid::commands::ingest;
use quaid::core::conversation::{
    extractor::{SlmClient, Worker},
    slm::SlmError,
    supersede::ResolvingFactWriter,
};
use quaid::core::db;
use quaid::mcp::server::{MemoryAddTurnInput, MemoryCloseSessionInput, QuaidServer};
use rusqlite::Connection;

struct Harness {
    _dir: tempfile::TempDir,
    vault_root: PathBuf,
    db_path: PathBuf,
    server: QuaidServer,
    inspect: Connection,
}

impl Harness {
    fn new() -> Self {
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
                "INSERT OR REPLACE INTO config(key, value) VALUES ('extraction.enabled', 'true')",
                [],
            )
            .unwrap();

        let inspect = db::open(db_path.to_str().unwrap()).unwrap();
        Self {
            _dir: dir,
            vault_root,
            db_path,
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct FactNodeSignature {
    kind: String,
    type_key: String,
    supersedes: Option<String>,
    is_head: bool,
}

fn run_quaid(db_path: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    command.arg("--db").arg(db_path).args(args);
    command.output().expect("run quaid")
}

fn markdown_files(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut entries = entries.flatten().map(|entry| entry.path()).collect::<Vec<_>>();
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

fn fact_structure(conn: &Connection) -> Vec<FactNodeSignature> {
    let mut stmt = conn
        .prepare(
            "SELECT
                type,
                COALESCE(
                    json_extract(frontmatter, '$.about'),
                    json_extract(frontmatter, '$.chose'),
                    json_extract(frontmatter, '$.what'),
                    ''
                ) AS type_key,
                json_extract(frontmatter, '$.supersedes') AS supersedes,
                CASE WHEN superseded_by IS NULL THEN 1 ELSE 0 END AS is_head
             FROM pages
             WHERE type IN ('decision', 'preference', 'fact', 'action_item')
             ORDER BY type, type_key, COALESCE(json_extract(frontmatter, '$.supersedes'), ''), slug",
        )
        .unwrap();

    stmt.query_map([], |row| {
        Ok(FactNodeSignature {
            kind: row.get(0)?,
            type_key: row.get(1)?,
            supersedes: row.get(2)?,
            is_head: row.get::<_, i64>(3)? != 0,
        })
    })
    .unwrap()
    .map(|row| row.unwrap())
    .collect()
}

fn head_partitions(conn: &Connection) -> BTreeSet<(String, String)> {
    let mut stmt = conn
        .prepare(
            "SELECT
                type,
                COALESCE(
                    json_extract(frontmatter, '$.about'),
                    json_extract(frontmatter, '$.chose'),
                    json_extract(frontmatter, '$.what'),
                    ''
                ) AS type_key
             FROM pages
             WHERE type IN ('decision', 'preference', 'fact', 'action_item')
               AND superseded_by IS NULL
             ORDER BY type, type_key",
        )
        .unwrap();

    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(|row| row.unwrap())
        .collect()
}

#[test]
fn force_reextract_keeps_structurally_equivalent_head_set_and_chain_shape() {
    let harness = Harness::new();

    for (ordinal, role, content) in [
        (1, "user", "I prefer Rust for systems work."),
        (2, "assistant", "Noted: Rust is the current preference."),
        (3, "user", "We chose SQLite for the local cache."),
        (4, "assistant", "Decision captured."),
    ] {
        harness
            .server
            .memory_add_turn(MemoryAddTurnInput {
                session_id: "idem-session".to_string(),
                role: role.to_string(),
                content: content.to_string(),
                timestamp: Some(format!("2026-05-05T09:0{ordinal}:00Z")),
                metadata: None,
                namespace: None,
            })
            .unwrap();
    }
    harness
        .server
        .memory_close_session(MemoryCloseSessionInput {
            session_id: "idem-session".to_string(),
            namespace: None,
        })
        .unwrap();

    let slm_output = r#"{"facts":[
        {"kind":"preference","about":"systems-language","strength":"high","summary":"The team prefers Rust for systems work."},
        {"kind":"decision","chose":"sqlite-local-cache","summary":"The team chose SQLite for the local cache."}
    ]}"#;
    let initial_worker = Worker::new(
        &harness.inspect,
        StubSlm::with_results([Ok(slm_output)]),
        ResolvingFactWriter,
    )
    .unwrap()
    .with_limits(Duration::from_millis(1), 128);
    initial_worker
        .process_next_job()
        .unwrap()
        .expect("initial extraction job");

    let extracted_root = harness.vault_root.join("extracted");
    let initial_files = markdown_files(&extracted_root);
    assert_eq!(initial_files.len(), 2, "initial extraction should write two fact files");
    for file in &initial_files {
        ingest::run(&harness.inspect, file.to_str().unwrap(), false).unwrap();
    }

    let initial_structure = fact_structure(&harness.inspect);
    let initial_heads = head_partitions(&harness.inspect);

    let output = run_quaid(&harness.db_path, &["extract", "idem-session", "--force"]);
    assert!(
        output.status.success(),
        "extract --force failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let replay_worker = Worker::new(
        &harness.inspect,
        StubSlm::with_results([Ok(slm_output)]),
        ResolvingFactWriter,
    )
    .unwrap()
    .with_limits(Duration::from_millis(1), 128);
    replay_worker
        .process_next_job()
        .unwrap()
        .expect("forced replay job");

    let replay_files = markdown_files(&extracted_root);
    assert_eq!(
        replay_files, initial_files,
        "force re-extraction should not grow the extracted fact set when outputs dedup"
    );

    let replay_structure = fact_structure(&harness.inspect);
    let replay_heads = head_partitions(&harness.inspect);
    assert_eq!(replay_heads, initial_heads);
    assert_eq!(replay_structure, initial_structure);
}
