#![cfg(feature = "online-model")]

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex, OnceLock,
};
use std::thread;
use std::time::Duration;

use quaid::core::conversation::{
    extractor::{SlmClient, Worker},
    model_lifecycle::load_model_from_local_cache,
    slm::SlmError,
    supersede::ResolvingFactWriter,
};
use quaid::core::db;
use quaid::mcp::server::{MemoryAddTurnInput, MemoryCloseSessionInput, QuaidServer};
use rusqlite::Connection;
use sha2::{Digest, Sha256};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> &'static Mutex<()> {
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvGuard {
    previous: Vec<(String, Option<String>)>,
}

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

#[derive(Clone)]
struct MockFile {
    content: Vec<u8>,
    etag: String,
}

struct MockModelServer {
    base_url: String,
    request_count: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl MockModelServer {
    fn start(repo_id: &str, files: HashMap<String, MockFile>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        listener.set_nonblocking(true).expect("set nonblocking");
        let address = listener.local_addr().expect("listener address");
        let base_url = format!("http://{}", address);
        let repo_id = repo_id.to_owned();
        let file_names = files.keys().cloned().collect::<Vec<_>>();
        let request_count = Arc::new(AtomicUsize::new(0));
        let request_counter = Arc::clone(&request_count);
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_signal = Arc::clone(&shutdown);

        let join_handle = thread::spawn(move || {
            while !shutdown_signal.load(Ordering::Relaxed) {
                let stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(_) => break,
                };
                let mut stream = stream;
                request_counter.fetch_add(1, Ordering::Relaxed);
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
                            .map(|name| serde_json::json!({"rfilename": name}))
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
            request_count,
            shutdown,
            join_handle: Some(join_handle),
        }
    }

    fn request_count(&self) -> usize {
        self.request_count.load(Ordering::Relaxed)
    }
}

impl Drop for MockModelServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

#[derive(Debug)]
struct LocalCacheSlm {
    responses: Mutex<VecDeque<String>>,
}

impl LocalCacheSlm {
    fn new(response: &str) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from([response.to_string()])),
        }
    }
}

impl SlmClient for LocalCacheSlm {
    fn infer(&self, alias: &str, _prompt: &str, _max_tokens: usize) -> Result<String, SlmError> {
        let _cache_dir = load_model_from_local_cache(alias)?;
        Ok(self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| "{\"facts\":[]}".to_string()))
    }
}

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

fn request_method(request: &str) -> &str {
    request.split_whitespace().next().unwrap_or("GET")
}

fn request_path(request: &str) -> Option<String> {
    let mut parts = request.lines().next()?.split_whitespace();
    let _method = parts.next()?;
    Some(parts.next()?.to_owned())
}

fn write_response(stream: &mut TcpStream, status: &str, headers: &[(String, String)], body: &[u8]) {
    let mut response = format!("HTTP/1.1 {status}\r\n");
    for (name, value) in headers {
        response.push_str(name);
        response.push_str(": ");
        response.push_str(value);
        response.push_str("\r\n");
    }
    response.push_str("Connection: close\r\n\r\n");
    stream.write_all(response.as_bytes()).expect("write headers");
    if !body.is_empty() {
        stream.write_all(body).expect("write body");
    }
}

fn mock_files() -> HashMap<String, MockFile> {
    let mut files = HashMap::new();
    for (name, content) in [
        ("config.json", br#"{"model_type":"phi3"}"#.as_slice()),
        ("tokenizer.json", br#"{"version":"1.0"}"#.as_slice()),
        ("model.safetensors", b"tiny-test-weights".as_slice()),
    ] {
        files.insert(
            name.to_owned(),
            MockFile {
                content: content.to_vec(),
                etag: format!("{:x}", Sha256::digest(content)),
            },
        );
    }
    files
}

fn open_test_db(path: &Path) -> Connection {
    let conn = db::open(path.to_str().unwrap()).unwrap();
    let vault_root = path.parent().unwrap().join("vault");
    fs::create_dir_all(&vault_root).unwrap();
    conn.execute(
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
    conn
}

fn run_quaid_with_env(db_path: &Path, args: &[&str], envs: &[(&str, String)]) -> Output {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    command.arg("--db").arg(db_path).args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().expect("run quaid")
}

fn markdown_files(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, files);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
                files.push(path);
            }
        }
    }

    let mut files = Vec::new();
    walk(root, &mut files);
    files.sort();
    files
}

#[test]
fn extraction_enable_followed_by_turn_capture_and_local_cache_extraction_stays_offline() {
    let _lock = env_lock().lock().unwrap_or_else(|error| error.into_inner());

    let repo_id = "test-org/test-airgap-model";
    let server = MockModelServer::start(repo_id, mock_files());
    let cache_root = tempfile::TempDir::new().unwrap();
    let db_dir = tempfile::TempDir::new().unwrap();
    let db_path = db_dir.path().join("memory.db");
    let conn = open_test_db(&db_path);
    conn.execute(
        "INSERT OR REPLACE INTO config(key, value) VALUES
            ('extraction.enabled', 'false'),
            ('extraction.model_alias', ?1)",
        [repo_id],
    )
    .unwrap();
    drop(conn);

    let _env = EnvGuard::set_all(&[
        ("QUAID_HF_BASE_URL", server.base_url.clone()),
        (
            "QUAID_MODEL_CACHE_DIR",
            cache_root.path().display().to_string(),
        ),
    ]);

    let enable = run_quaid_with_env(&db_path, &["extraction", "enable"], &[]);
    assert!(
        enable.status.success(),
        "extraction enable failed: {}",
        String::from_utf8_lossy(&enable.stderr)
    );
    let requests_after_enable = server.request_count();
    assert!(requests_after_enable > 0, "enable should have populated cache via HTTP");

    let server_conn = db::open(db_path.to_str().unwrap()).unwrap();
    let inspect = db::open(db_path.to_str().unwrap()).unwrap();
    let app = QuaidServer::new(server_conn);
    app.memory_add_turn(MemoryAddTurnInput {
        session_id: "airgap-session".to_string(),
        role: "user".to_string(),
        content: "Remember that the app must stay fully airgapped after setup.".to_string(),
        timestamp: Some("2026-05-05T09:00:00Z".to_string()),
        metadata: None,
        namespace: None,
    })
    .unwrap();
    app.memory_close_session(MemoryCloseSessionInput {
        session_id: "airgap-session".to_string(),
        namespace: None,
    })
    .unwrap();

    let worker = Worker::new(
        &inspect,
        LocalCacheSlm::new(
            r#"{"facts":[{"kind":"fact","about":"airgap-mode","summary":"Extraction must stay fully airgapped after model setup."}]}"#,
        ),
        ResolvingFactWriter,
    )
    .unwrap()
    .with_limits(Duration::from_millis(1), 128);
    worker
        .process_next_job()
        .unwrap()
        .expect("pending extraction job");

    assert_eq!(
        server.request_count(),
        requests_after_enable,
        "no outbound model-network calls should occur after enable completes"
    );
    let extracted = markdown_files(&db_dir.path().join("vault").join("extracted"));
    assert_eq!(extracted.len(), 1, "expected one extracted fact file");
}
