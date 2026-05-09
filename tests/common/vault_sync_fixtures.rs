#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    dead_code,
    unreachable_pub,
    reason = "test fixtures legitimately panic on setup failure; pub helpers are shared across `tests/vault_sync_*.rs` files but unreachable from non-test crates; `dead_code` because individual test files only use a subset of the helpers"
)]

//! Shared test fixtures for `tests/vault_sync_*.rs` integration tests.
//!
//! Mirrors a subset of the inline helpers that previously lived inside
//! `src/core/vault_sync.rs::tests` — only the helpers that the moved
//! public-API tests need. White-box helpers (e.g. `startup_recovery_sentinel_count`,
//! `writer_side_sentinel_path`, `writer_side_tempfile_path`,
//! `insert_page_with_actual_file_state`, `stored_file_state`,
//! `actual_file_stat`) stay inline because they reference private items and per
//! the test-organization spec visibility cannot be widened.

use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use quaid::core::db;
use quaid::core::markdown;
use quaid::core::vault_sync::{build_restore_manifest_for_directory, collection_recovery_dir};

pub static ENV_MUTATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub fn env_mutation_lock() -> &'static Mutex<()> {
    ENV_MUTATION_LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(all(unix, target_os = "linux"))]
pub fn secure_runtime_root() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::TempDir::new().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

pub struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    pub fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        #[expect(
            unsafe_code,
            reason = "std::env::set_var is unsafe on Rust 1.81+; tests are single-threaded under ENV_MUTATION_LOCK so the data-race precondition is upheld"
        )]
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }

    pub fn clear(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        #[expect(
            unsafe_code,
            reason = "std::env::remove_var is unsafe on Rust 1.81+; serialised via ENV_MUTATION_LOCK"
        )]
        unsafe {
            std::env::remove_var(key);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        #[expect(
            unsafe_code,
            reason = "std::env::set_var/remove_var are unsafe on Rust 1.81+; the guard owns the same lock window as the constructor"
        )]
        unsafe {
            if let Some(value) = self.previous.as_ref() {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}

pub fn open_test_db() -> Connection {
    db::open(":memory:").unwrap()
}

pub fn open_test_db_file() -> (tempfile::TempDir, String, Connection) {
    let dir = tempfile::TempDir::new().unwrap();
    // Canonicalize: SQLite resolves /var → /private/var on macOS when reading
    // PRAGMA database_list, so we must store the canonical path as the test's
    // identifier so production-side lookups (e.g. test-hook keys, runtime
    // registry keys) match.
    let canonical_dir = fs::canonicalize(dir.path()).unwrap();
    let db_path = canonical_dir.join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    (dir, db_path.display().to_string(), conn)
}

pub fn insert_collection(conn: &Connection, name: &str, root_path: &Path) -> i64 {
    // Production paths reach the `collections.root_path` column via add() which
    // canonicalizes via fs::canonicalize. Tests insert directly here, so canonicalize
    // here too; otherwise macOS's /var ↔ /private/var symlink causes mismatches with
    // production-side lookups (live-owner checks, watcher path matching).
    let root_path = fs::canonicalize(root_path).unwrap_or_else(|_| root_path.to_path_buf());
    conn.execute(
        "INSERT INTO collections (name, root_path, state, writable, is_write_target)
         VALUES (?1, ?2, 'active', 1, 0)",
        params![name, root_path.display().to_string()],
    )
    .unwrap();
    conn.last_insert_rowid()
}

pub fn insert_collection_with_id(
    conn: &Connection,
    collection_id: i64,
    name: &str,
    root_path: &Path,
) -> i64 {
    conn.execute(
        "INSERT INTO collections (id, name, root_path, state, writable, is_write_target)
         VALUES (?1, ?2, ?3, 'active', 1, 0)",
        params![collection_id, name, root_path.display().to_string()],
    )
    .unwrap();
    collection_id
}

pub fn insert_page_with_raw_import(
    conn: &Connection,
    collection_id: i64,
    slug: &str,
    uuid: &str,
    compiled_truth: &str,
    raw_bytes: &[u8],
    relative_path: &str,
) -> i64 {
    let frontmatter_json = std::str::from_utf8(raw_bytes)
        .ok()
        .map(|s| {
            let (fm, _) = markdown::parse_frontmatter(s);
            serde_json::to_string(&fm).unwrap_or_else(|_| "{}".to_owned())
        })
        .unwrap_or_else(|| "{}".to_owned());
    conn.execute(
        "INSERT INTO pages
             (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
         VALUES (?1, ?2, ?3, 'concept', ?2, '', ?4, '', ?5, '', '', 1)",
        params![collection_id, slug, uuid, compiled_truth, frontmatter_json],
    )
    .unwrap();
    let page_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO raw_imports (page_id, import_id, is_active, raw_bytes, file_path)
         VALUES (?1, ?2, 1, ?3, ?4)",
        params![
            page_id,
            Uuid::now_v7().to_string(),
            raw_bytes,
            relative_path
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO file_state (collection_id, relative_path, page_id, mtime_ns, ctime_ns, size_bytes, inode, sha256)
         VALUES (?1, ?2, ?3, 1, 1, ?4, 1, ?5)",
        params![collection_id, relative_path, page_id, raw_bytes.len() as i64, sha256_hex(raw_bytes)],
    )
    .unwrap();
    page_id
}

pub fn write_restore_file(root: &Path, relative_path: &str, bytes: &[u8]) {
    let path = root.join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

pub fn production_vault_sync_source() -> String {
    // Concatenate the production halves of every .rs file under the
    // vault_sync directory tree. Source-introspection tests (the
    // `*_source_*` tests in tests/vault_sync_*.rs) call this helper
    // because the items they grep for can land in any submodule —
    // mod.rs, restore.rs, ipc/handler.rs, ipc/socket.rs,
    // embedding.rs, watcher.rs, etc. The exact file does not matter
    // for those tests; only that the function definition or call
    // site is present somewhere in production source.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("core")
        .join("vault_sync");
    let mut sources: Vec<(String, String)> = Vec::new();
    visit_rs_files(&root, &mut sources);
    // Sort by path so the concatenation order is deterministic
    // across machines and the indices source-introspection tests
    // compute via `.find()` are stable.
    sources.sort_by(|a, b| a.0.cmp(&b.0));
    let mut combined = String::new();
    for (_path, source) in sources {
        // Truncate at the inline `#[cfg(test)] mod tests { ... }`
        // block at the end of the file (if present). We anchor on
        // `mod tests` rather than any `#[cfg(test)]` marker because
        // production code legitimately uses cfg(test) on individual
        // items (e.g., `#[cfg(test)] Variant` inside an enum) — a
        // plain `rfind("#[cfg(test)]")` over-truncates.
        let production = source
            .rfind("\nmod tests {")
            .and_then(|mod_tests_start| {
                source[..mod_tests_start]
                    .rfind("#[cfg(test)]")
                    .map(|cfg_start| &source[..cfg_start])
            })
            .unwrap_or(source.as_str());
        combined.push_str(production);
        combined.push('\n');
    }
    combined
}

fn visit_rs_files(dir: &Path, out: &mut Vec<(String, String)>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit_rs_files(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            if let Ok(source) = std::fs::read_to_string(&path) {
                out.push((path.display().to_string(), source));
            }
        }
    }
}

pub fn manifest_json_for_directory(root: &Path) -> String {
    serde_json::to_string(&build_restore_manifest_for_directory(root).unwrap()).unwrap()
}

#[cfg(unix)]
pub fn wait_for_collection_update<T, F>(
    db_path: &str,
    collection_id: i64,
    timeout: Duration,
    read: F,
) -> T
where
    F: Fn(&Connection, i64) -> Option<T>,
{
    let started = Instant::now();
    loop {
        let verify = Connection::open(db_path).unwrap();
        if let Some(result) = read(&verify, collection_id) {
            return result;
        }
        drop(verify);
        assert!(
            started.elapsed() < timeout,
            "timed out waiting for collection_id={collection_id} after {:?}",
            timeout
        );
        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(unix)]
pub fn create_startup_recovery_sentinel(recovery_root: &Path, collection_id: i64, name: &str) {
    let dir = collection_recovery_dir(recovery_root, collection_id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(name), b"dirty").unwrap();
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}
