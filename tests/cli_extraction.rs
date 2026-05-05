mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use quaid::core::{
    conversation::format,
    db,
    types::{ConversationFile, ConversationFrontmatter, ConversationStatus, Turn, TurnRole},
};
use rusqlite::{params, Connection};

#[cfg(feature = "online-model")]
use quaid::core::conversation::model_lifecycle::cache_dir_for_alias;
#[cfg(feature = "online-model")]
use sha2::{Digest, Sha256};
#[cfg(feature = "online-model")]
use std::collections::HashMap;
#[cfg(feature = "online-model")]
use std::io::{Read, Write};
#[cfg(feature = "online-model")]
use std::net::{TcpListener, TcpStream};
#[cfg(feature = "online-model")]
use std::sync::{Arc, Mutex, OnceLock};
#[cfg(feature = "online-model")]
use std::thread;
#[cfg(feature = "online-model")]
use std::time::Duration;

fn open_test_db(path: &Path) -> Connection {
    let conn = db::open(path.to_str().expect("utf-8 db path")).expect("open test db");
    let vault_root = path.parent().expect("db parent").join("vault");
    fs::create_dir_all(&vault_root).expect("create vault root");
    conn.execute(
        "UPDATE collections
         SET root_path = ?1,
             writable = 1,
             is_write_target = 1,
             state = 'active'
         WHERE id = 1",
        [vault_root.display().to_string()],
    )
    .expect("configure default collection");
    conn
}

fn run_quaid(db_path: &Path, args: &[&str]) -> Output {
    run_quaid_with_env(db_path, args, &[])
}

fn run_quaid_with_env(db_path: &Path, args: &[&str], envs: &[(String, String)]) -> Output {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    command.arg("--db").arg(db_path).args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().expect("run quaid")
}

fn sqlite_relative_timestamp(conn: &Connection, modifier: &str) -> String {
    conn.query_row(
        "SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now', ?1)",
        [modifier],
        |row| row.get(0),
    )
    .expect("query timestamp")
}

fn write_conversation_file(
    root: &Path,
    relative_path: &str,
    session_id: &str,
    last_turn_at: &str,
    last_extracted_at: Option<&str>,
    status: ConversationStatus,
) {
    let absolute = root.join(relative_path.replace('/', std::path::MAIN_SEPARATOR_STR));
    fs::create_dir_all(absolute.parent().expect("conversation parent")).expect("create dirs");
    let file = ConversationFile {
        frontmatter: ConversationFrontmatter {
            file_type: "conversation".to_string(),
            session_id: session_id.to_string(),
            date: relative_path
                .split('/')
                .nth_back(1)
                .expect("date segment")
                .to_string(),
            started_at: last_turn_at.to_string(),
            status,
            closed_at: None,
            last_extracted_at: last_extracted_at.map(str::to_string),
            last_extracted_turn: 1,
        },
        turns: vec![Turn {
            ordinal: 1,
            role: TurnRole::User,
            timestamp: last_turn_at.to_string(),
            content: format!("turn for {session_id}"),
            metadata: None,
        }],
    };
    fs::write(absolute, format::render(&file)).expect("write conversation");
}

#[test]
fn extraction_status_reports_queue_sessions_and_failed_jobs_shape() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_test_db(&db_path);
    let vault_root = dir.path().join("vault");

    conn.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES
            ('extraction.enabled', 'true'),
            ('extraction.idle_close_ms', '60000'),
            ('extraction.model_alias', 'phi-3.5-mini')",
        [],
    )
    .unwrap();

    let active_turn_at = sqlite_relative_timestamp(&conn, "-10 seconds");
    let active_extracted_at = sqlite_relative_timestamp(&conn, "-5 seconds");
    let namespaced_turn_at = sqlite_relative_timestamp(&conn, "-20 seconds");
    let stale_turn_at = sqlite_relative_timestamp(&conn, "-2 minutes");
    let failed_at = sqlite_relative_timestamp(&conn, "-1 hour");
    let old_failed_at = sqlite_relative_timestamp(&conn, "-2 days");

    write_conversation_file(
        &vault_root,
        "conversations/2026-05-05/s1.md",
        "s1",
        &active_turn_at,
        Some(&active_extracted_at),
        ConversationStatus::Open,
    );
    write_conversation_file(
        &vault_root,
        "alpha/conversations/2026-05-05/s2.md",
        "s2",
        &namespaced_turn_at,
        None,
        ConversationStatus::Open,
    );
    write_conversation_file(
        &vault_root,
        "conversations/2026-05-05/stale.md",
        "stale",
        &stale_turn_at,
        None,
        ConversationStatus::Open,
    );

    let long_error = format!("JSON parse failure at offset 247: {}", "x".repeat(240));
    conn.execute(
        "INSERT INTO extraction_queue
             (session_id, conversation_path, trigger_kind, enqueued_at, scheduled_for, attempts, last_error, status)
         VALUES
             ('pending-one', 'conversations/2026-05-05/s1.md', 'manual', ?1, ?1, 0, NULL, 'pending'),
             ('pending-two', 'alpha/conversations/2026-05-05/s2.md', 'debounce', ?1, ?1, 0, NULL, 'pending'),
             ('failed-run', 'conversations/2026-05-05/s1.md', 'manual', ?1, ?1, 3, ?2, 'failed'),
             ('failed-old', 'conversations/2026-05-05/stale.md', 'manual', ?3, ?3, 3, 'old failure', 'failed')",
        params![failed_at, long_error, old_failed_at],
    )
    .unwrap();
    drop(conn);

    let output = run_quaid(&db_path, &["extraction", "status"]);
    assert!(
        output.status.success(),
        "status failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Extraction enabled: yes"));
    assert!(stdout.contains("Queue depth: pending=2 running=0 failed_last_24h=1"));
    assert!(stdout.contains("Active sessions (idle window 1m):"));
    assert!(stdout.contains("  - s1 "));
    assert!(stdout.contains("  - alpha/s2 "));
    assert!(!stdout.contains("  - stale "));
    assert!(stdout.contains("Failed jobs (last 24h):"));
    assert!(stdout.contains("failed-run"));
    assert!(stdout.contains("attempts: 3"));
    assert!(stdout.contains("quaid extract <session> --force"));

    let failed_line = stdout
        .lines()
        .find(|line| line.contains("failed-run"))
        .expect("failed job line");
    assert!(
        failed_line.chars().count() < 260,
        "failed job line should truncate long last_error: {failed_line}"
    );
}

#[cfg(feature = "online-model")]
fn env_lock() -> &'static Mutex<()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(feature = "online-model")]
struct EnvGuard {
    previous: Vec<(String, Option<String>)>,
}

#[cfg(feature = "online-model")]
impl EnvGuard {
    fn set_all(pairs: &[(&str, String)]) -> Self {
        let previous = pairs
            .iter()
            .map(|(key, value)| {
                let prior = std::env::var(key).ok();
                std::env::set_var(key, value);
                ((*key).to_owned(), prior)
            })
            .collect();
        Self { previous }
    }
}

#[cfg(feature = "online-model")]
impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in self.previous.drain(..) {
            if let Some(value) = value {
                std::env::set_var(&key, value);
            } else {
                std::env::remove_var(&key);
            }
        }
    }
}

#[cfg(feature = "online-model")]
#[derive(Clone)]
struct MockFile {
    content: Vec<u8>,
    etag: String,
}

#[cfg(feature = "online-model")]
struct MockModelServer {
    base_url: String,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
    join_handle: Option<thread::JoinHandle<()>>,
}

#[cfg(feature = "online-model")]
impl MockModelServer {
    fn start(repo_id: &str, files: HashMap<String, MockFile>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        listener.set_nonblocking(true).expect("set nonblocking");
        let base_url = format!(
            "http://{}",
            listener.local_addr().expect("listener address")
        );
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let shutdown_signal = Arc::clone(&shutdown);
        let repo_id = repo_id.to_string();
        let file_names = files.keys().cloned().collect::<Vec<_>>();

        let join_handle = thread::spawn(move || {
            while !shutdown_signal.load(std::sync::atomic::Ordering::Relaxed) {
                let stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(_) => break,
                };
                let mut stream = stream;
                let request = read_request(&mut stream);
                if request.is_empty() {
                    continue;
                }
                let Some(path) = request_path(&request) else {
                    write_response(&mut stream, "400 Bad Request", &[], &[]);
                    continue;
                };
                let method = request_method(&request);

                if path == format!("/api/models/{repo_id}") {
                    let body = serde_json::json!({
                        "siblings": file_names
                            .iter()
                            .map(|name| serde_json::json!({ "rfilename": name }))
                            .collect::<Vec<_>>()
                    })
                    .to_string()
                    .into_bytes();
                    write_response(
                        &mut stream,
                        "200 OK",
                        &[("Content-Type".to_owned(), "application/json".to_owned())],
                        if method == "HEAD" { &[] } else { &body },
                    );
                    continue;
                }

                let prefix = format!("/{repo_id}/resolve/main/");
                if let Some(file_name) = path.strip_prefix(&prefix) {
                    if let Some(file) = files.get(file_name) {
                        write_response(
                            &mut stream,
                            "200 OK",
                            &[
                                (
                                    "Content-Type".to_owned(),
                                    "application/octet-stream".to_owned(),
                                ),
                                ("ETag".to_owned(), format!("\"{}\"", file.etag)),
                                ("Content-Length".to_owned(), file.content.len().to_string()),
                            ],
                            if method == "HEAD" { &[] } else { &file.content },
                        );
                    } else {
                        write_response(&mut stream, "404 Not Found", &[], &[]);
                    }
                    continue;
                }

                write_response(&mut stream, "404 Not Found", &[], &[]);
            }
        });

        Self {
            base_url,
            shutdown,
            join_handle: Some(join_handle),
        }
    }
}

#[cfg(feature = "online-model")]
impl Drop for MockModelServer {
    fn drop(&mut self) {
        self.shutdown
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(feature = "online-model")]
fn read_request(stream: &mut TcpStream) -> String {
    let mut buffer = [0_u8; 4096];
    let mut request = Vec::new();
    loop {
        let read = match stream.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(error) => panic!("read request: {error}"),
        };
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&request).into_owned()
}

#[cfg(feature = "online-model")]
fn request_method(request: &str) -> &str {
    request.split_whitespace().next().unwrap_or("GET")
}

#[cfg(feature = "online-model")]
fn request_path(request: &str) -> Option<String> {
    let mut parts = request.lines().next()?.split_whitespace();
    let _method = parts.next()?;
    Some(parts.next()?.to_owned())
}

#[cfg(feature = "online-model")]
fn write_response(stream: &mut TcpStream, status: &str, headers: &[(String, String)], body: &[u8]) {
    let mut response = format!("HTTP/1.1 {status}\r\n");
    for (name, value) in headers {
        response.push_str(name);
        response.push_str(": ");
        response.push_str(value);
        response.push_str("\r\n");
    }
    response.push_str("Connection: close\r\n\r\n");
    stream
        .write_all(response.as_bytes())
        .expect("write response headers");
    if !body.is_empty() {
        stream.write_all(body).expect("write response body");
    }
}

#[cfg(feature = "online-model")]
fn raw_model_files(bad_model_etag: bool) -> HashMap<String, MockFile> {
    let mut files = HashMap::new();
    for (name, content) in [
        ("config.json", br#"{"model_type":"phi3"}"#.as_slice()),
        ("tokenizer.json", br#"{"version":"1.0"}"#.as_slice()),
        ("model.safetensors", b"tiny-test-weights".as_slice()),
    ] {
        let actual_sha = format!("{:x}", Sha256::digest(content));
        let etag = if bad_model_etag && name == "model.safetensors" {
            "0".repeat(64)
        } else {
            actual_sha
        };
        files.insert(
            name.to_string(),
            MockFile {
                content: content.to_vec(),
                etag,
            },
        );
    }
    files
}

#[cfg(feature = "online-model")]
#[test]
fn extraction_enable_downloads_model_and_flips_flag_only_on_success() {
    let _env_guard = env_lock().lock().unwrap();
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_test_db(&db_path);
    let alias = "test-org/raw-model";
    conn.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES
            ('extraction.enabled', 'false'),
            ('extraction.model_alias', ?1)",
        [alias],
    )
    .unwrap();
    drop(conn);

    let cache_root = dir.path().join("model-cache");
    let server = MockModelServer::start(alias, raw_model_files(false));
    let envs = vec![
        (
            "QUAID_MODEL_CACHE_DIR".to_string(),
            cache_root.display().to_string(),
        ),
        ("QUAID_HF_BASE_URL".to_string(), server.base_url.clone()),
    ];
    let output = run_quaid_with_env(&db_path, &["extraction", "enable"], &envs);
    assert!(
        output.status.success(),
        "enable failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let _env = EnvGuard::set_all(&[("QUAID_MODEL_CACHE_DIR", cache_root.display().to_string())]);
    let cache_dir = cache_dir_for_alias(alias).expect("cache dir");
    assert!(cache_dir.join("manifest.json").is_file());

    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    let enabled: String = conn
        .query_row(
            "SELECT value FROM config WHERE key = 'extraction.enabled'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(enabled, "true");
}

#[cfg(feature = "online-model")]
#[test]
fn extraction_enable_leaves_flag_false_when_integrity_check_fails() {
    let _env_guard = env_lock().lock().unwrap();
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_test_db(&db_path);
    let alias = "test-org/raw-model";
    conn.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES
            ('extraction.enabled', 'false'),
            ('extraction.model_alias', ?1)",
        [alias],
    )
    .unwrap();
    drop(conn);

    let cache_root = dir.path().join("model-cache");
    let server = MockModelServer::start(alias, raw_model_files(true));
    let envs = vec![
        (
            "QUAID_MODEL_CACHE_DIR".to_string(),
            cache_root.display().to_string(),
        ),
        ("QUAID_HF_BASE_URL".to_string(), server.base_url.clone()),
    ];
    let output = run_quaid_with_env(&db_path, &["extraction", "enable"], &envs);
    assert!(!output.status.success(), "enable should fail on bad ETag");

    let _env = EnvGuard::set_all(&[("QUAID_MODEL_CACHE_DIR", cache_root.display().to_string())]);
    let cache_dir = cache_dir_for_alias(alias).expect("cache dir");
    assert!(
        !cache_dir.exists(),
        "failed enable must not leave a promoted cache"
    );

    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    let enabled: String = conn
        .query_row(
            "SELECT value FROM config WHERE key = 'extraction.enabled'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(enabled, "false");
}

#[cfg(feature = "online-model")]
#[test]
fn model_pull_caches_model_without_flipping_extraction_flag() {
    let _env_guard = env_lock().lock().unwrap();
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_test_db(&db_path);
    conn.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES ('extraction.enabled', 'false')",
        [],
    )
    .unwrap();
    drop(conn);

    let alias = "test-org/raw-model";
    let cache_root = dir.path().join("model-cache");
    let server = MockModelServer::start(alias, raw_model_files(false));
    let envs = vec![
        (
            "QUAID_MODEL_CACHE_DIR".to_string(),
            cache_root.display().to_string(),
        ),
        ("QUAID_HF_BASE_URL".to_string(), server.base_url.clone()),
    ];
    let output = run_quaid_with_env(&db_path, &["model", "pull", alias], &envs);
    assert!(
        output.status.success(),
        "model pull failed: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let _env = EnvGuard::set_all(&[("QUAID_MODEL_CACHE_DIR", cache_root.display().to_string())]);
    let cache_dir = cache_dir_for_alias(alias).expect("cache dir");
    assert!(cache_dir.join("manifest.json").is_file());

    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    let enabled: String = conn
        .query_row(
            "SELECT value FROM config WHERE key = 'extraction.enabled'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(enabled, "false");
}
