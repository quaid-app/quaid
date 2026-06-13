//! Conversation-turn writer.
//!
//! Owns the path from "a turn happened" to "the right day-file on
//! disk has been appended atomically": resolves the memory root
//! collection, takes a per-session in-process mutex plus an on-disk
//! `flock` so concurrent writers don't interleave, parses any
//! existing day-file to derive the next turn ordinal, and either
//! appends to the existing file or creates a new one. Also exposes
//! the explicit close path (`close_session`, `close_session_if_idle`)
//! that flips a day-file's `status` to `closed`.
//!
//! See also: `super::format` for the on-disk Markdown shape this
//! module writes, `super::idle_close` for the silence-driven caller
//! of `close_session_if_idle`, and `crate::core::collections` for
//! the write-target resolution underneath `resolve_memory_root`.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value as JsonValue;
use thiserror::Error;

use crate::core::collections::{self, Collection};
use crate::core::conversation::format::{
    self, ConversationFormatError, ConversationPathInfo, MemoryLocation,
};
use crate::core::conversation::idle_close;
use crate::core::db;
use crate::core::namespace;
use crate::core::types::{
    CloseSessionResult, ConversationFile, ConversationFrontmatter, ConversationStatus, Turn,
    TurnRole, TurnWriteResult,
};

const DEDICATED_COLLECTION_SUFFIX: &str = "-memory";
const DEDICATED_ROOT_SUFFIX: &str = "-quaid-memory";

static SESSION_LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();

/// Errors surfaced by turn-writer operations.
#[derive(Debug, Error)]
pub enum TurnWriteError {
    /// The conversation Markdown could not be parsed or rendered.
    #[error("conversation format error: {0}")]
    Format(#[from] ConversationFormatError),

    /// Underlying SQLite failure when reading config or collections.
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// File-system failure while reading or writing a day-file or
    /// lock file.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Session identifier was empty, contained path traversal, or
    /// otherwise failed validation.
    #[error("invalid session id: {message}")]
    InvalidSessionId {
        /// Human-readable description of why the id is invalid.
        message: String,
    },

    /// Required config (memory.location, write-target collection) is
    /// missing or invalid.
    #[error("config error: {message}")]
    Config {
        /// Human-readable description of the offending config value.
        message: String,
    },

    /// Caller attempted to append to or close a session whose latest
    /// day-file is already in `status: closed`.
    #[error("conflict: session `{session_id}` is already closed")]
    SessionClosed {
        /// Session identifier that was already closed.
        session_id: String,
    },

    /// Caller asked to close a session for which no day-file exists.
    #[error("session not found: {session_id}")]
    SessionNotFound {
        /// Session identifier the caller asked about.
        session_id: String,
    },
}

/// Resolved on-disk root where conversation day-files will be written
/// for the active vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRoot {
    /// Database id of the collection backing this root.
    pub collection_id: i64,
    /// Human-readable collection name (used for diagnostics and for
    /// deriving dedicated-collection names).
    pub collection_name: String,
    /// Absolute filesystem path to the collection root.
    pub root_path: PathBuf,
    /// Which `memory.location` policy resolved this root.
    pub location: MemoryLocation,
}

/// Append one turn to the session's day-file for `timestamp`,
/// creating the day-file when none exists yet, and stamp the session
/// as active for the idle-close tracker.
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
    // Prefer the cached per-session cursor (max_ordinal + latest status) so we
    // avoid the O(session) day-file scan on every turn. The cache is rebuilt
    // from disk (the source of truth) when no row exists yet.
    let snapshot = cached_or_scanned_snapshot(conn, &root.root_path, namespace, session_id)?;
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

    // Refresh the cursor cache to reflect the turn we just wrote. The new turn's
    // day-file is the latest by construction (its date is >= every prior file),
    // so its status is the session's latest status.
    upsert_cached_snapshot(
        conn,
        namespace,
        session_id,
        &SessionSnapshot {
            max_ordinal: ordinal,
            latest_status: Some(ConversationStatus::Open),
            latest_date: Some(path_info.date.clone()),
        },
    )?;

    idle_close::record_turn(&database_path(conn)?, namespace, session_id);

    Ok(TurnWriteResult {
        turn_id: format!("{session_id}:{ordinal}"),
        ordinal,
        conversation_path: slash_path(&path_info.relative_path),
    })
}

/// Unconditionally close `session_id` by flipping the latest
/// day-file's `status` to `closed` and stamping `closed_at`. Idempotent:
/// closing an already-closed session reports `newly_closed: false`.
pub fn close_session(
    conn: &Connection,
    session_id: &str,
    namespace: Option<&str>,
) -> Result<CloseSessionResult, TurnWriteError> {
    close_session_internal(conn, session_id, namespace, None)?.ok_or_else(|| {
        TurnWriteError::Config {
            message: "close_session unexpectedly skipped".to_owned(),
        }
    })
}

/// Close `session_id` only if the idle-close tracker reports it has
/// been silent for at least `idle_for` as of `now`; returns `Ok(None)`
/// when the session is still considered active.
pub fn close_session_if_idle(
    conn: &Connection,
    session_id: &str,
    namespace: Option<&str>,
    idle_for: Duration,
    now: Instant,
) -> Result<Option<CloseSessionResult>, TurnWriteError> {
    close_session_internal(conn, session_id, namespace, Some((idle_for, now)))
}

fn close_session_internal(
    conn: &Connection,
    session_id: &str,
    namespace: Option<&str>,
    idle_guard: Option<(Duration, Instant)>,
) -> Result<Option<CloseSessionResult>, TurnWriteError> {
    namespace::validate_optional_namespace(namespace).map_err(|error| TurnWriteError::Config {
        message: error.to_string(),
    })?;
    validate_session_id(session_id)?;
    let root = resolve_memory_root(conn)?;
    let db_path = database_path(conn)?;
    let lock = session_lock(&root, namespace, session_id)?;
    let _guard = lock.lock().map_err(|_| TurnWriteError::Config {
        message: "session lock poisoned".to_owned(),
    })?;
    let _file_lock =
        SessionFileLock::acquire(&session_lock_path(&root.root_path, namespace, session_id))?;

    if let Some((idle_for, now)) = idle_guard {
        if !idle_close::is_idle_at(&db_path, namespace, session_id, now, idle_for) {
            return Ok(None);
        }
    }

    let Some((full_path, relative_path, mut conversation)) =
        latest_session_file(&root.root_path, namespace, session_id)?
    else {
        return Err(TurnWriteError::SessionNotFound {
            session_id: session_id.to_owned(),
        });
    };

    if conversation.frontmatter.status == ConversationStatus::Closed {
        idle_close::clear_session(&db_path, namespace, session_id);
        let closed_at = conversation
            .frontmatter
            .closed_at
            .clone()
            .unwrap_or_else(|| conversation.frontmatter.started_at.clone());
        return Ok(Some(CloseSessionResult {
            closed_at,
            conversation_path: slash_path(&relative_path),
            newly_closed: false,
        }));
    }

    let closed_at = current_timestamp(conn)?;
    conversation.frontmatter.status = ConversationStatus::Closed;
    conversation.frontmatter.closed_at = Some(closed_at.clone());
    write_conversation_file(&full_path, &conversation)?;
    idle_close::clear_session(&db_path, namespace, session_id);

    // Keep the append-cursor cache coherent: a future `append_turn` that opens
    // a new day-file must see the session as Closed without rescanning disk.
    // Preserve the cached ordinal/date when present, else rebuild from disk.
    let mut snapshot = read_cached_snapshot(conn, namespace, session_id)?.map_or_else(
        || session_snapshot(&root.root_path, namespace, session_id),
        Ok,
    )?;
    snapshot.latest_status = Some(ConversationStatus::Closed);
    upsert_cached_snapshot(conn, namespace, session_id, &snapshot)?;

    Ok(Some(CloseSessionResult {
        closed_at,
        conversation_path: slash_path(&relative_path),
        newly_closed: true,
    }))
}

/// Resolve the active write-target collection plus the
/// `memory.location` policy into the concrete on-disk root where
/// conversation day-files belong, creating a dedicated-collection
/// directory on first call when that policy is selected.
pub fn resolve_memory_root(conn: &Connection) -> Result<MemoryRoot, TurnWriteError> {
    let location = MemoryLocation::from_config(
        &db::read_config_value_or(conn, "memory.location", "vault-subdir").map_err(|error| {
            TurnWriteError::Config {
                message: error.to_string(),
            }
        })?,
    )?;
    // Opening a database no longer provisions the default `~/.quaid/vault`
    // root on disk; heal the empty-root placeholder here, at the moment a
    // write actually needs a usable write-target root.
    db::provision_default_collection_root(conn).map_err(|error| TurnWriteError::Config {
        message: error.to_string(),
    })?;
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
            closed_at: None,
            last_extracted_at: None,
            last_extracted_turn: 0,
        },
        turns: vec![turn.clone()],
    };
    write_conversation_file(full_path, &conversation)
}

fn append_turn_block(full_path: &Path, turn: &Turn) -> Result<(), TurnWriteError> {
    let mut file = OpenOptions::new().append(true).open(full_path)?;
    writeln!(file)?;
    writeln!(file, "{}", format::TURN_BOUNDARY)?;
    writeln!(file)?;
    file.write_all(format::render_turn_block(turn).as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn write_conversation_file(
    full_path: &Path,
    conversation: &ConversationFile,
) -> Result<(), TurnWriteError> {
    let rendered = format::render(conversation);
    let mut file = File::create(full_path)?;
    file.write_all(rendered.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

struct SessionSnapshot {
    max_ordinal: i64,
    latest_status: Option<ConversationStatus>,
    latest_date: Option<String>,
}

/// Returns the session cursor (max ordinal + latest status), preferring the
/// `conversation_sessions` cache and falling back to a full day-file scan when
/// no cached row exists yet. A cache miss seeds the cache so the next turn
/// reads it instead of rescanning. This is what makes `append_turn` O(1) in the
/// number of prior day-files rather than O(session-length).
fn cached_or_scanned_snapshot(
    conn: &Connection,
    root_path: &Path,
    namespace: Option<&str>,
    session_id: &str,
) -> Result<SessionSnapshot, TurnWriteError> {
    if let Some(cached) = read_cached_snapshot(conn, namespace, session_id)? {
        return Ok(cached);
    }
    let scanned = session_snapshot(root_path, namespace, session_id)?;
    upsert_cached_snapshot(conn, namespace, session_id, &scanned)?;
    Ok(scanned)
}

fn read_cached_snapshot(
    conn: &Connection,
    namespace: Option<&str>,
    session_id: &str,
) -> Result<Option<SessionSnapshot>, TurnWriteError> {
    let namespace = namespace.unwrap_or("");
    conn.query_row(
        "SELECT max_ordinal, latest_status, latest_date
         FROM conversation_sessions
         WHERE namespace = ?1 AND session_id = ?2",
        rusqlite::params![namespace, session_id],
        |row| {
            let max_ordinal: i64 = row.get(0)?;
            let status_text: Option<String> = row.get(1)?;
            let latest_date: Option<String> = row.get(2)?;
            Ok((max_ordinal, status_text, latest_date))
        },
    )
    .optional()?
    .map(|(max_ordinal, status_text, latest_date)| {
        let latest_status = status_text
            .map(|text| text.parse::<ConversationStatus>())
            .transpose()
            .map_err(|message| TurnWriteError::Config { message })?;
        Ok(SessionSnapshot {
            max_ordinal,
            latest_status,
            latest_date,
        })
    })
    .transpose()
}

fn upsert_cached_snapshot(
    conn: &Connection,
    namespace: Option<&str>,
    session_id: &str,
    snapshot: &SessionSnapshot,
) -> Result<(), TurnWriteError> {
    let namespace = namespace.unwrap_or("");
    conn.execute(
        "INSERT INTO conversation_sessions
             (namespace, session_id, max_ordinal, latest_status, latest_date, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
         ON CONFLICT(namespace, session_id) DO UPDATE SET
             max_ordinal = excluded.max_ordinal,
             latest_status = excluded.latest_status,
             latest_date = excluded.latest_date,
             updated_at = excluded.updated_at",
        rusqlite::params![
            namespace,
            session_id,
            snapshot.max_ordinal,
            snapshot
                .latest_status
                .as_ref()
                .map(|status| status.as_str()),
            snapshot.latest_date,
        ],
    )?;
    Ok(())
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
            latest_date: None,
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
        latest_date,
    })
}

fn latest_session_file(
    root_path: &Path,
    namespace: Option<&str>,
    session_id: &str,
) -> Result<Option<(PathBuf, PathBuf, ConversationFile)>, TurnWriteError> {
    let mut conversations_root = root_path.to_path_buf();
    if let Some(namespace) = namespace.filter(|value| !value.is_empty()) {
        conversations_root.push(namespace);
    }
    conversations_root.push("conversations");

    if !conversations_root.exists() {
        return Ok(None);
    }

    let mut latest: Option<(String, PathBuf, PathBuf, ConversationFile)> = None;
    for entry in fs::read_dir(&conversations_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let relative_path = entry
            .path()
            .strip_prefix(root_path)
            .map_err(|error| TurnWriteError::Config {
                message: format!("failed to resolve conversation path: {error}"),
            })?
            .join(format!("{session_id}.md"));
        let candidate = root_path.join(&relative_path);
        if !candidate.exists() {
            continue;
        }
        let conversation = format::parse(&candidate)?;
        let date = conversation.frontmatter.date.clone();
        match latest.as_ref() {
            Some((current_date, ..)) if current_date >= &date => {}
            _ => {
                latest = Some((date, candidate, relative_path, conversation));
            }
        }
    }

    Ok(latest.map(|(_, full_path, relative_path, conversation)| {
        (full_path, relative_path, conversation)
    }))
}

fn current_timestamp(conn: &Connection) -> Result<String, TurnWriteError> {
    conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| {
        row.get(0)
    })
    .map_err(TurnWriteError::from)
}

fn database_path(conn: &Connection) -> Result<String, TurnWriteError> {
    conn.query_row(
        "SELECT file FROM pragma_database_list WHERE name = 'main'",
        [],
        |row| row.get::<_, String>(0),
    )
    .map_err(TurnWriteError::from)
}

/// Run `op` while holding the same in-process per-session mutex and on-disk
/// `SessionFileLock` that [`append_turn`] holds, so admin paths (e.g.
/// `extract --force` cursor reset) cannot race a concurrent turn append on
/// the same session's day-files.
pub fn with_session_locks<F, R, E>(
    root: &MemoryRoot,
    namespace: Option<&str>,
    session_id: &str,
    op: F,
) -> Result<R, E>
where
    F: FnOnce() -> Result<R, E>,
    E: From<TurnWriteError>,
{
    let lock = session_lock(root, namespace, session_id).map_err(E::from)?;
    let _guard = lock
        .lock()
        .map_err(|_| TurnWriteError::Config {
            message: "session lock poisoned".to_owned(),
        })
        .map_err(E::from)?;
    let _file_lock =
        SessionFileLock::acquire(&session_lock_path(&root.root_path, namespace, session_id))
            .map_err(E::from)?;
    op()
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

    #[expect(
        unsafe_code,
        reason = "POSIX flock() is a syscall with no safe Rust wrapper; we pass a valid file descriptor obtained from File::as_raw_fd"
    )]
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

    #[expect(
        unsafe_code,
        reason = "POSIX flock() is a syscall with no safe Rust wrapper; we pass a valid file descriptor obtained from File::as_raw_fd"
    )]
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

    #[expect(
        unsafe_code,
        reason = "extern \"system\" declaration of the Win32 LockFile API; signature mirrors the documented kernel32 export"
    )]
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
        #[expect(
            unsafe_code,
            reason = "Win32 LockFile is a syscall; we pass a valid handle from File::as_raw_handle"
        )]
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

    #[expect(
        unsafe_code,
        reason = "extern \"system\" declaration of the Win32 UnlockFile API; signature mirrors the documented kernel32 export"
    )]
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

    #[expect(
        unsafe_code,
        reason = "Win32 UnlockFile is a syscall; we pass a valid handle from File::as_raw_handle"
    )]
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
    fn append_turn_returns_format_error_when_existing_file_is_malformed() {
        let vault_root = tempfile::TempDir::new().unwrap();
        let (_db_dir, conn) = configured_connection(vault_root.path());
        let conversation_dir = vault_root.path().join("conversations").join("2026-05-03");
        fs::create_dir_all(&conversation_dir).unwrap();
        let conversation_path = conversation_dir.join("session-bad.md");
        let original = concat!(
            "---\n",
            "type: conversation\n",
            "session_id: session-bad\n",
            "date: 2026-05-03\n",
            "started_at: 2026-05-03T09:14:22Z\n",
            "status: open\n",
            "last_extracted_at: null\n",
            "last_extracted_turn: 0\n",
            "---\n\n",
            "## Turn nope · user · 2026-05-03T09:14:22Z\n\n",
            "broken\n"
        );
        fs::write(&conversation_path, original).unwrap();

        let error = append_turn(
            &conn,
            "session-bad",
            TurnRole::Assistant,
            "should fail",
            "2026-05-03T09:15:00Z",
            None,
            None,
        )
        .unwrap_err();

        assert!(matches!(error, TurnWriteError::Format(_)));
        assert_eq!(fs::read_to_string(&conversation_path).unwrap(), original);
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

    #[test]
    fn close_session_marks_latest_day_file_closed_and_is_idempotent() {
        let vault_root = tempfile::TempDir::new().unwrap();
        let (_db_dir, conn) = configured_connection(vault_root.path());
        append_turn(
            &conn,
            "session-close",
            TurnRole::User,
            "first day",
            "2026-05-03T23:59:00Z",
            None,
            None,
        )
        .unwrap();
        append_turn(
            &conn,
            "session-close",
            TurnRole::Assistant,
            "second day",
            "2026-05-04T00:01:00Z",
            None,
            None,
        )
        .unwrap();

        let first = close_session(&conn, "session-close", None).unwrap();
        let second = close_session(&conn, "session-close", None).unwrap();
        let latest_path = vault_root
            .path()
            .join("conversations")
            .join("2026-05-04")
            .join("session-close.md");
        let latest = format::parse(&latest_path).unwrap();

        assert!(first.newly_closed);
        assert_eq!(
            first.conversation_path,
            "conversations/2026-05-04/session-close.md"
        );
        assert_eq!(latest.frontmatter.status, ConversationStatus::Closed);
        assert_eq!(
            latest.frontmatter.closed_at.as_deref(),
            Some(first.closed_at.as_str())
        );
        assert!(!second.newly_closed);
        assert_eq!(second.closed_at, first.closed_at);
    }

    #[test]
    fn close_session_returns_namespaced_path_for_namespaced_session() {
        let vault_root = tempfile::TempDir::new().unwrap();
        let (_db_dir, conn) = configured_connection(vault_root.path());
        append_turn(
            &conn,
            "session-alpha",
            TurnRole::User,
            "hello",
            "2026-05-03T09:14:22Z",
            None,
            Some("alpha"),
        )
        .unwrap();

        let closed = close_session(&conn, "session-alpha", Some("alpha")).unwrap();

        assert_eq!(
            closed.conversation_path,
            "alpha/conversations/2026-05-03/session-alpha.md"
        );
        assert!(format::parse(
            &vault_root
                .path()
                .join("alpha")
                .join("conversations")
                .join("2026-05-03")
                .join("session-alpha.md"),
        )
        .unwrap()
        .frontmatter
        .closed_at
        .is_some());
    }

    #[test]
    fn close_session_returns_not_found_for_unknown_session() {
        let vault_root = tempfile::TempDir::new().unwrap();
        let (_db_dir, conn) = configured_connection(vault_root.path());

        let error = close_session(&conn, "missing-session", None).unwrap_err();

        assert!(matches!(
            error,
            TurnWriteError::SessionNotFound { session_id } if session_id == "missing-session"
        ));
    }

    #[test]
    fn close_session_uses_started_at_for_legacy_closed_file_without_closed_at() {
        let vault_root = tempfile::TempDir::new().unwrap();
        let (_db_dir, conn) = configured_connection(vault_root.path());
        let conversation_dir = vault_root.path().join("conversations").join("2026-05-03");
        fs::create_dir_all(&conversation_dir).unwrap();
        fs::write(
            conversation_dir.join("session-legacy.md"),
            concat!(
                "---\n",
                "type: conversation\n",
                "session_id: session-legacy\n",
                "date: 2026-05-03\n",
                "started_at: 2026-05-03T09:14:22Z\n",
                "status: closed\n",
                "last_extracted_at: null\n",
                "last_extracted_turn: 1\n",
                "---\n\n",
                "## Turn 1 · user · 2026-05-03T09:14:22Z\n\n",
                "done\n"
            ),
        )
        .unwrap();

        let result = close_session(&conn, "session-legacy", None).unwrap();

        assert_eq!(result.closed_at, "2026-05-03T09:14:22Z");
        assert!(!result.newly_closed);
    }
}
