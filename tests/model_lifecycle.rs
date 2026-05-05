#![cfg(feature = "online-model")]

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use quaid::core::{
    conversation::model_lifecycle::{
        cache_dir_for_alias, cached_model_status, download_model, resolve_model_alias,
        NoopProgressReporter, ProgressReporter,
    },
    db,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};
use std::thread;
use std::time::Duration;

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
    shutdown: Arc<AtomicBool>,
    join_handle: Option<thread::JoinHandle<()>>,
}

struct SeedCacheOnFirstDownloadReporter {
    cache_dir: PathBuf,
    requested_alias: String,
    repo_id: String,
    files: HashMap<String, MockFile>,
    seeded: bool,
}

impl ProgressReporter for SeedCacheOnFirstDownloadReporter {
    fn file_started(&mut self, _file_name: &str, _bytes_total: Option<u64>) {
        if self.seeded {
            return;
        }
        seed_valid_cache(
            &self.cache_dir,
            &self.requested_alias,
            &self.repo_id,
            &self.files,
        );
        self.seeded = true;
    }
}

impl MockModelServer {
    fn start(repo_id: &str, revision: &str, files: HashMap<String, MockFile>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let address = listener.local_addr().expect("listener address");
        let base_url = format!("http://{}", address);
        let repo_id = repo_id.to_owned();
        let revision = revision.to_owned();
        let file_names = files.keys().cloned().collect::<Vec<_>>();
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
                let request = read_request(&mut stream);
                if request.is_empty() {
                    continue;
                }
                let Some(path) = request_path(&request) else {
                    write_response(&mut stream, "400 Bad Request", &[], &[]);
                    continue;
                };
                let method = request_method(&request);

                if path == format!("/api/models/{repo_id}")
                    || path == format!("/api/models/{repo_id}/revision/{revision}")
                {
                    let body = serde_json::json!({
                        "siblings": file_names.iter().map(|name| serde_json::json!({"rfilename": name})).collect::<Vec<_>>()
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

                let prefix = format!("/{repo_id}/resolve/{revision}/");
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

impl Drop for MockModelServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
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
    stream
        .write_all(response.as_bytes())
        .expect("write response headers");
    if !body.is_empty() {
        stream.write_all(body).expect("write response body");
    }
}

fn mock_files(with_bad_model_hash: bool) -> HashMap<String, MockFile> {
    let mut files = HashMap::new();
    for (name, content) in [
        ("config.json", br#"{"model_type":"phi3"}"#.as_slice()),
        ("tokenizer.json", br#"{"version":"1.0"}"#.as_slice()),
        ("model.safetensors", b"tiny-test-weights".as_slice()),
    ] {
        let actual_sha = format!("{:x}", Sha256::digest(content));
        let etag = if with_bad_model_hash && name == "model.safetensors" {
            "0000000000000000000000000000000000000000000000000000000000000000".to_owned()
        } else {
            actual_sha
        };
        files.insert(
            name.to_owned(),
            MockFile {
                content: content.to_vec(),
                etag,
            },
        );
    }
    files
}

fn seed_valid_cache(
    cache_dir: &Path,
    requested_alias: &str,
    repo_id: &str,
    files: &HashMap<String, MockFile>,
) {
    std::fs::create_dir_all(cache_dir).expect("create seeded cache dir");
    let mut manifest_files = files
        .iter()
        .map(|(path, file)| {
            std::fs::write(cache_dir.join(path), &file.content).expect("write seeded file");
            serde_json::json!({
                "path": path,
                "sha256": format!("{:x}", Sha256::digest(&file.content)),
                "verified_from_source": true
            })
        })
        .collect::<Vec<_>>();
    manifest_files.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
    let manifest = serde_json::json!({
        "requested_alias": requested_alias,
        "repo_id": repo_id,
        "revision": serde_json::Value::Null,
        "files": manifest_files
    });
    std::fs::write(
        cache_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("serialize seeded manifest"),
    )
    .expect("write seeded manifest");
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

fn open_db(path: &Path) -> Connection {
    db::open(path.to_str().expect("utf8 db path")).expect("open db")
}

#[test]
fn resolve_model_alias_maps_gemma_3_4b() {
    let resolved = resolve_model_alias("gemma-3-4b").expect("resolve alias");
    assert_eq!(resolved.repo_id, "google/gemma-3-4b-it");
    assert_eq!(
        resolved.revision.as_deref(),
        Some("093f9f388b31de276ce2de164bdc2081324b9767")
    );
}

#[test]
fn download_model_installs_manifest_and_recovers_stale_cache() {
    let _lock = env_lock().lock().unwrap_or_else(|error| error.into_inner());
    let repo_id = "org/test-model";
    let revision = "main";
    let server = MockModelServer::start(repo_id, revision, mock_files(false));
    let cache_root = tempfile::TempDir::new().expect("cache root");
    let stale_cache_dir = cache_root.path().join("org-test-model");
    std::fs::create_dir_all(&stale_cache_dir).expect("create stale cache");
    std::fs::write(stale_cache_dir.join("manifest.json"), b"{\"bad\":true}")
        .expect("write stale manifest");

    let _env = EnvGuard::set_all(&[
        ("QUAID_HF_BASE_URL", server.base_url.clone()),
        (
            "QUAID_MODEL_CACHE_DIR",
            cache_root.path().display().to_string(),
        ),
    ]);

    let mut reporter = NoopProgressReporter;
    let cache_dir = download_model(repo_id, &mut reporter).expect("download model");

    assert_eq!(cache_dir, stale_cache_dir);
    assert!(cache_dir.join("manifest.json").is_file());
    assert!(cache_dir.join("config.json").is_file());
    assert!(cache_dir.join("tokenizer.json").is_file());
    assert!(cache_dir.join("model.safetensors").is_file());
    let leftovers = std::fs::read_dir(cache_root.path())
        .expect("read cache root")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("download"))
        .collect::<Vec<_>>();
    assert!(
        leftovers.is_empty(),
        "temporary dirs left behind: {leftovers:?}"
    );

    let status = cached_model_status(repo_id).expect("cache status");
    assert!(status.is_cached);
    assert!(status.verified);
}

#[test]
fn download_model_rejects_bad_integrity_and_cleans_partial_cache() {
    let _lock = env_lock().lock().unwrap_or_else(|error| error.into_inner());
    let repo_id = "org/test-model";
    let revision = "main";
    let server = MockModelServer::start(repo_id, revision, mock_files(true));
    let cache_root = tempfile::TempDir::new().expect("cache root");
    let _env = EnvGuard::set_all(&[
        ("QUAID_HF_BASE_URL", server.base_url.clone()),
        (
            "QUAID_MODEL_CACHE_DIR",
            cache_root.path().display().to_string(),
        ),
    ]);

    let mut reporter = NoopProgressReporter;
    let error = download_model(repo_id, &mut reporter).expect_err("integrity failure");
    let message = error.to_string();
    assert!(message.contains("integrity check failed"));

    let cache_dir = cache_dir_for_alias(repo_id).expect("cache dir");
    assert!(
        !cache_dir.exists(),
        "cache dir should not survive a failed install"
    );
    let leftovers = std::fs::read_dir(cache_root.path())
        .expect("read cache root")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(
        leftovers.is_empty(),
        "partial downloads should be removed, found {leftovers:?}"
    );
}

#[test]
fn download_model_succeeds_when_another_writer_populates_the_cache_first() {
    let _lock = env_lock().lock().unwrap_or_else(|error| error.into_inner());
    let repo_id = "org/test-model";
    let revision = "main";
    let files = mock_files(false);
    let server = MockModelServer::start(repo_id, revision, files.clone());
    let cache_root = tempfile::TempDir::new().expect("cache root");
    let _env = EnvGuard::set_all(&[
        ("QUAID_HF_BASE_URL", server.base_url.clone()),
        (
            "QUAID_MODEL_CACHE_DIR",
            cache_root.path().display().to_string(),
        ),
    ]);

    let cache_dir = cache_dir_for_alias(repo_id).expect("cache dir");
    let mut reporter = SeedCacheOnFirstDownloadReporter {
        cache_dir: cache_dir.clone(),
        requested_alias: repo_id.to_owned(),
        repo_id: repo_id.to_owned(),
        files,
        seeded: false,
    };
    let result =
        download_model(repo_id, &mut reporter).expect("download should treat race as success");
    assert_eq!(result, cache_dir);

    let status = cached_model_status(repo_id).expect("cache status");
    assert!(status.is_cached);
    assert!(status.verified);
}

#[test]
fn cli_model_pull_caches_without_flipping_extraction_flag() {
    let _lock = env_lock().lock().unwrap_or_else(|error| error.into_inner());
    let repo_id = "org/test-model";
    let revision = "main";
    let server = MockModelServer::start(repo_id, revision, mock_files(false));
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = dir.path().join("memory.db");
    db::init(
        db_path.to_str().expect("utf8 db path"),
        &quaid::core::inference::default_model(),
    )
    .expect("init db");

    let envs = vec![
        ("QUAID_HF_BASE_URL", server.base_url.clone()),
        (
            "QUAID_MODEL_CACHE_DIR",
            dir.path().join("cache").display().to_string(),
        ),
    ];
    let output = run_quaid_with_env(&db_path, &["model", "pull", repo_id], &envs);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Model cached at"));

    let conn = open_db(&db_path);
    let enabled = quaid::core::db::read_config_value_or(&conn, "extraction.enabled", "false")
        .expect("read extraction flag");
    assert_eq!(enabled, "false");
}

#[test]
fn cli_extraction_enable_then_disable_updates_flag() {
    let _lock = env_lock().lock().unwrap_or_else(|error| error.into_inner());
    let repo_id = "org/test-model";
    let revision = "main";
    let server = MockModelServer::start(repo_id, revision, mock_files(false));
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = dir.path().join("memory.db");
    db::init(
        db_path.to_str().expect("utf8 db path"),
        &quaid::core::inference::default_model(),
    )
    .expect("init db");
    let conn = open_db(&db_path);
    conn.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES ('extraction.model_alias', ?1)",
        [repo_id],
    )
    .expect("set extraction.model_alias");
    drop(conn);

    let envs = vec![
        ("QUAID_HF_BASE_URL", server.base_url.clone()),
        (
            "QUAID_MODEL_CACHE_DIR",
            dir.path().join("cache").display().to_string(),
        ),
    ];
    let enable_output = run_quaid_with_env(&db_path, &["extraction", "enable"], &envs);
    assert!(
        enable_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&enable_output.stdout),
        String::from_utf8_lossy(&enable_output.stderr)
    );
    assert!(String::from_utf8_lossy(&enable_output.stdout).contains("Extraction enabled: yes"));

    let conn = open_db(&db_path);
    let enabled = quaid::core::db::read_config_value_or(&conn, "extraction.enabled", "false")
        .expect("read extraction flag");
    assert_eq!(enabled, "true");
    drop(conn);

    let disable_output = run_quaid_with_env(&db_path, &["extraction", "disable"], &envs);
    assert!(
        disable_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&disable_output.stdout),
        String::from_utf8_lossy(&disable_output.stderr)
    );

    let conn = open_db(&db_path);
    let enabled = quaid::core::db::read_config_value_or(&conn, "extraction.enabled", "false")
        .expect("read extraction flag");
    assert_eq!(enabled, "false");
}
