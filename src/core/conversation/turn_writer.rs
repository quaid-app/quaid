use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use rusqlite::{params, Connection};
use serde_json::Value as JsonValue;
use thiserror::Error;

use crate::core::collections::{self, Collection};
use crate::core::conversation::format::{
    self, ConversationFormatError, ConversationPathInfo, MemoryLocation,
};
use crate::core::db;
use crate::core::namespace;
use crate::core::types::{
    ConversationFile, ConversationFrontmatter, ConversationStatus, Turn, TurnRole, TurnWriteResult,
};

const DEDICATED_COLLECTION_SUFFIX: &str = "-memory";
const DEDICATED_ROOT_SUFFIX: &str = "-quaid-memory";

static SESSION_LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();

#[derive(Debug, Error)]
pub enum TurnWriteError {
    #[error("conversation format error: {0}")]
    Format(#[from] ConversationFormatError),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid session id: {message}")]
    InvalidSessionId { message: String },

    #[error("config error: {message}")]
    Config { message: String },

    #[error("conflict: session `{session_id}` is already closed")]
    SessionClosed { session_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRoot {
    pub collection_id: i64,
    pub collection_name: String,
    pub root_path: PathBuf,
    pub location: MemoryLocation,
}

pub fn append_turn(
    conn: &Connection,
    session_id: &str,
    role: TurnRole,
    content: &str,
    timestamp: &str,
    metadata: Option<JsonValue>,
    namespace: Option<&str>,
) -> Result<TurnWriteResult, TurnWriteError> {
    namespace::validate_optional_namespace(namespace).map_err(|error| TurnWriteError::Config {
        message: error.to_string(),
    })?;
    validate_session_id(session_id)?;
    let root = resolve_memory_root(conn)?;
    let lock = session_lock(&root, namespace, session_id)?;
    let _guard = lock.lock().map_err(|_| TurnWriteError::Config {
        message: "session lock poisoned".to_owned(),
    })?;
    let _file_lock =
        SessionFileLock::acquire(&session_lock_path(&root.root_path, namespace, session_id))?;
    maybe_hold_session_lock_for_tests()?;

    let path_info = format::conversation_path_for(namespace, session_id, timestamp)?;
    let full_path = root.root_path.join(&path_info.relative_path);
    let snapshot = session_snapshot(&root.root_path, namespace, session_id)?;
    if !full_path.exists() && snapshot.latest_status == Some(ConversationStatus::Closed) {
        return Err(TurnWriteError::SessionClosed {
            session_id: session_id.to_owned(),
        });
    }
    let ordinal = snapshot.max_ordinal + 1;
    let turn = Turn {
        ordinal,
        role,
        timestamp: timestamp.to_owned(),
        content: content.to_owned(),
        metadata,
    };

    if full_path.exists() {
        let existing = format::parse(&full_path)?;
        if existing.frontmatter.status == ConversationStatus::Closed {
            return Err(TurnWriteError::SessionClosed {
                session_id: session_id.to_owned(),
            });
        }
        append_turn_block(&full_path, &turn)?;
    } else {
        write_new_file(&full_path, &path_info, session_id, timestamp, &turn)?;
    }

    Ok(TurnWriteResult {
        turn_id: format!("{session_id}:{ordinal}"),
        ordinal,
        conversation_path: slash_path(&path_info.relative_path),
    })
}

pub fn resolve_memory_root(conn: &Connection) -> Result<MemoryRoot, TurnWriteError> {
    let location = MemoryLocation::from_config(
        &db::read_config_value_or(conn, "memory.location", "vault-subdir").map_err(|error| {
            TurnWriteError::Config {
                message: error.to_string(),
            }
        })?,
    )?;
    let write_target = collections::get_write_target(conn)
        .map_err(|error| TurnWriteError::Config {
            message: error.to_string(),
        })?
        .filter(|collection| !collection.root_path.trim().is_empty() && collection.writable)
        .ok_or_else(|| TurnWriteError::Config {
            message: "memory storage requires a writable write-target collection root".to_owned(),
        })?;

    match location {
        MemoryLocation::VaultSubdir => Ok(MemoryRoot {
            collection_id: write_target.id,
            collection_name: write_target.name,
            root_path: PathBuf::from(write_target.root_path),
            location,
        }),
        MemoryLocation::DedicatedCollection => {
            let collection = ensure_dedicated_collection(conn, &write_target)?;
            Ok(MemoryRoot {
                collection_id: collection.id,
                collection_name: collection.name,
                root_path: PathBuf::from(collection.root_path),
                location,
            })
        }
    }
}

fn ensure_dedicated_collection(
    conn: &Connection,
    write_target: &Collection,
) -> Result<Collection, TurnWriteError> {
    let dedicated_name = format!("{}{}", write_target.name, DEDICATED_COLLECTION_SUFFIX);
    if let Some(existing) =
        collections::get_by_name(conn, &dedicated_name).map_err(|error| TurnWriteError::Config {
            message: error.to_string(),
        })?
    {
        if !existing.writable || existing.root_path.trim().is_empty() {
            return Err(TurnWriteError::Config {
                message: format!(
                    "memory.location collection `{}` is not writable",
                    existing.name
                ),
            });
        }
        fs::create_dir_all(&existing.root_path)?;
        return Ok(existing);
    }

    let main_root = PathBuf::from(&write_target.root_path);
    let dedicated_root = derive_dedicated_root(&main_root, &write_target.name);
    fs::create_dir_all(&dedicated_root)?;

    conn.execute(
        "INSERT INTO collections (name, root_path, state, writable, is_write_target, needs_full_sync)
         VALUES (?1, ?2, 'active', 1, 0, 1)",
        params![dedicated_name, dedicated_root.display().to_string()],
    )?;

    collections::get_by_name(conn, &dedicated_name)
        .map_err(|error| TurnWriteError::Config {
            message: error.to_string(),
        })?
        .ok_or_else(|| TurnWriteError::Config {
            message: format!("failed to create dedicated collection `{dedicated_name}`"),
        })
}

fn derive_dedicated_root(main_root: &Path, collection_name: &str) -> PathBuf {
    let stem = main_root
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(collection_name);
    let parent = main_root.parent().unwrap_or(main_root);
    parent.join(format!("{stem}{DEDICATED_ROOT_SUFFIX}"))
}

fn write_new_file(
    full_path: &Path,
    path_info: &ConversationPathInfo,
    session_id: &str,
    timestamp: &str,
    turn: &Turn,
) -> Result<(), TurnWriteError> {
    let parent = full_path.parent().ok_or_else(|| TurnWriteError::Config {
        message: format!("conversation path has no parent: {}", full_path.display()),
    })?;
    fs::create_dir_all(parent)?;
    let conversation = ConversationFile {
        frontmatter: ConversationFrontmatter {
            file_type: "conversation".to_owned(),
            session_id: session_id.to_owned(),
            date: path_info.date.clone(),
            started_at: timestamp.to_owned(),
            status: ConversationStatus::Open,
            last_extracted_at: None,
            last_extracted_turn: 0,
        },
        turns: vec![turn.clone()],
    };
    let rendered = format::render(&conversation);
    let mut file = File::create(full_path)?;
    file.write_all(rendered.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn append_turn_block(full_path: &Path, turn: &Turn) -> Result<(), TurnWriteError> {
    let mut file = OpenOptions::new().append(true).open(full_path)?;
    file.write_all(b"\n---\n\n")?;
    file.write_all(format::render_turn_block(turn).as_bytes())?;
    file.sync_all()?;
    Ok(())
}

struct SessionSnapshot {
    max_ordinal: i64,
    latest_status: Option<ConversationStatus>,
}

fn session_snapshot(
    root_path: &Path,
    namespace: Option<&str>,
    session_id: &str,
) -> Result<SessionSnapshot, TurnWriteError> {
    let mut conversations_root = root_path.to_path_buf();
    if let Some(namespace) = namespace.filter(|value| !value.is_empty()) {
        conversations_root.push(namespace);
    }
    conversations_root.push("conversations");

    if !conversations_root.exists() {
        return Ok(SessionSnapshot {
            max_ordinal: 0,
            latest_status: None,
        });
    }

    let mut max_ordinal = 0_i64;
    let mut latest_date: Option<String> = None;
    let mut latest_status = None;
    for entry in fs::read_dir(conversations_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let candidate = entry.path().join(format!("{session_id}.md"));
        if !candidate.exists() {
            continue;
        }
        let conversation = format::parse(&candidate)?;
        if let Some(turn) = conversation.turns.iter().max_by_key(|turn| turn.ordinal) {
            max_ordinal = max_ordinal.max(turn.ordinal);
        }
        if latest_date
            .as_deref()
            .map(|current| conversation.frontmatter.date.as_str() > current)
            .unwrap_or(true)
        {
            latest_date = Some(conversation.frontmatter.date.clone());
            latest_status = Some(conversation.frontmatter.status.clone());
        }
    }

    Ok(SessionSnapshot {
        max_ordinal,
        latest_status,
    })
}

fn session_lock(
    root: &MemoryRoot,
    namespace: Option<&str>,
    session_id: &str,
) -> Result<Arc<Mutex<()>>, TurnWriteError> {
    let key = format!(
        "{}|{}|{}",
        root.root_path.display(),
        namespace.unwrap_or(""),
        session_id
    );
    let mut locks = SESSION_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| TurnWriteError::Config {
            message: "session lock registry poisoned".to_owned(),
        })?;
    Ok(locks
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone())
}

fn session_lock_path(root_path: &Path, namespace: Option<&str>, session_id: &str) -> PathBuf {
    let mut path = root_path.to_path_buf();
    if let Some(namespace) = namespace.filter(|value| !value.is_empty()) {
        path.push(namespace);
    }
    path.push("conversations");
    path.push(".locks");
    for segment in session_id.split('/') {
        path.push(segment);
    }
    path.set_extension("lock");
    path
}

struct SessionFileLock {
    file: File,
}

impl SessionFileLock {
    fn acquire(path: &Path) -> Result<Self, TurnWriteError> {
        let parent = path.parent().ok_or_else(|| TurnWriteError::Config {
            message: format!("session lock path has no parent: {}", path.display()),
        })?;
        fs::create_dir_all(parent)?;
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        lock_file(&file)?;
        Ok(Self { file })
    }
}

impl Drop for SessionFileLock {
    fn drop(&mut self) {
        let _ = unlock_file(&self.file);
    }
}

fn maybe_hold_session_lock_for_tests() -> Result<(), TurnWriteError> {
    if let Some(signal_path) = std::env::var_os("QUAID_TEST_APPEND_TURN_LOCK_SIGNAL") {
        fs::write(signal_path, b"locked")?;
    }
    if let Some(raw_ms) = std::env::var_os("QUAID_TEST_APPEND_TURN_HOLD_MS") {
        let hold_ms =
            raw_ms
                .to_string_lossy()
                .parse::<u64>()
                .map_err(|_| TurnWriteError::Config {
                    message: format!(
                        "invalid QUAID_TEST_APPEND_TURN_HOLD_MS value: {}",
                        raw_ms.to_string_lossy()
                    ),
                })?;
        std::thread::sleep(Duration::from_millis(hold_ms));
    }
    Ok(())
}

#[cfg(unix)]
fn lock_file(file: &File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn unlock_file(file: &File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn lock_file(file: &File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;

    const ERROR_LOCK_VIOLATION: i32 = 33;
    const ERROR_SHARING_VIOLATION: i32 = 32;

    #[link(name = "kernel32")]
    extern "system" {
        fn LockFile(
            h_file: *mut std::ffi::c_void,
            offset_low: u32,
            offset_high: u32,
            bytes_low: u32,
            bytes_high: u32,
        ) -> i32;
    }

    loop {
        let locked = unsafe { LockFile(file.as_raw_handle(), 0, 0, 1, 0) };
        if locked != 0 {
            return Ok(());
        }

        let error = std::io::Error::last_os_error();
        match error.raw_os_error() {
            Some(ERROR_LOCK_VIOLATION | ERROR_SHARING_VIOLATION) => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => return Err(error),
        }
    }
}

#[cfg(windows)]
fn unlock_file(file: &File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;

    #[link(name = "kernel32")]
    extern "system" {
        fn UnlockFile(
            h_file: *mut std::ffi::c_void,
            offset_low: u32,
            offset_high: u32,
            bytes_low: u32,
            bytes_high: u32,
        ) -> i32;
    }

    let unlocked = unsafe { UnlockFile(file.as_raw_handle(), 0, 0, 1, 0) };
    if unlocked != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn validate_session_id(session_id: &str) -> Result<(), TurnWriteError> {
    collections::validate_relative_path(session_id).map_err(|error| {
        TurnWriteError::InvalidSessionId {
            message: error.to_string(),
        }
    })
}

fn slash_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured_connection(root: &Path) -> (tempfile::TempDir, Connection) {
        let db_dir = tempfile::TempDir::new().unwrap();
        let db_path = db_dir.path().join("memory.db");
        let conn = db::open(db_path.to_str().unwrap()).unwrap();
        conn.execute(
            "UPDATE collections
             SET root_path = ?1,
                 state = 'active'
             WHERE id = 1",
            [root.display().to_string()],
        )
        .unwrap();
        (db_dir, conn)
    }

    #[test]
    fn resolve_memory_root_uses_write_target_in_vault_subdir_mode() {
        let vault_root = tempfile::TempDir::new().unwrap();
        let (_db_dir, conn) = configured_connection(vault_root.path());

        let root = resolve_memory_root(&conn).unwrap();

        assert_eq!(root.location, MemoryLocation::VaultSubdir);
        assert_eq!(root.root_path, vault_root.path());
    }

    #[test]
    fn resolve_memory_root_creates_dedicated_collection_once() {
        let vault_root = tempfile::TempDir::new().unwrap();
        let (_db_dir, conn) = configured_connection(vault_root.path());
        conn.execute(
            "UPDATE config SET value = 'dedicated-collection' WHERE key = 'memory.location'",
            [],
        )
        .unwrap();

        let first = resolve_memory_root(&conn).unwrap();
        let second = resolve_memory_root(&conn).unwrap();
        let expected_name = format!("{}{}", "default", DEDICATED_COLLECTION_SUFFIX);

        assert_eq!(first.location, MemoryLocation::DedicatedCollection);
        assert_eq!(first.root_path, second.root_path);
        assert!(first
            .root_path
            .display()
            .to_string()
            .contains(DEDICATED_ROOT_SUFFIX));
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM collections WHERE name = ?1",
                [expected_name],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn append_turn_appends_second_turn_to_same_day_file() {
        let vault_root = tempfile::TempDir::new().unwrap();
        let (_db_dir, conn) = configured_connection(vault_root.path());

        append_turn(
            &conn,
            "session-1",
            TurnRole::User,
            "first",
            "2026-05-03T09:14:22Z",
            None,
            None,
        )
        .unwrap();
        append_turn(
            &conn,
            "session-1",
            TurnRole::Assistant,
            "second",
            "2026-05-03T09:15:00Z",
            Some(serde_json::json!({"importance":"high"})),
            None,
        )
        .unwrap();

        let rendered = fs::read_to_string(
            vault_root
                .path()
                .join("conversations")
                .join("2026-05-03")
                .join("session-1.md"),
        )
        .unwrap();

        assert!(rendered.contains("## Turn 1 · user · 2026-05-03T09:14:22Z"));
        assert!(rendered.contains("## Turn 2 · assistant · 2026-05-03T09:15:00Z"));
        assert!(rendered.contains("\"importance\": \"high\""));
    }

    #[test]
    fn validate_session_id_rejects_path_traversal() {
        let error = validate_session_id("../bad").unwrap_err();

        assert!(error.to_string().contains("invalid session id"));
    }

    #[test]
    fn resolve_memory_root_rejects_unknown_memory_location() {
        let vault_root = tempfile::TempDir::new().unwrap();
        let (_db_dir, conn) = configured_connection(vault_root.path());
        conn.execute(
            "UPDATE config SET value = 'mystery-mode' WHERE key = 'memory.location'",
            [],
        )
        .unwrap();

        let error = resolve_memory_root(&conn).unwrap_err();

        assert!(error.to_string().contains("unsupported memory.location"));
    }
}
