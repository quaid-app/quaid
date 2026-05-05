use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use rusqlite::Connection;
use thiserror::Error;

use crate::core::conversation::{
    queue::{self, ExtractionQueueError},
    turn_writer::{self, TurnWriteError},
};
use crate::core::db;
use crate::core::types::ExtractionTriggerKind;

const DEFAULT_IDLE_CLOSE_MS: i64 = 60_000;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SessionKey {
    namespace: Option<String>,
    session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdleCloseResult {
    pub namespace: Option<String>,
    pub session_id: String,
    pub conversation_path: String,
    pub scheduled_for: String,
    pub newly_closed: bool,
}

#[derive(Debug, Error)]
pub enum IdleCloseError {
    #[error("queue error: {0}")]
    Queue(#[from] ExtractionQueueError),

    #[error("turn write error: {0}")]
    TurnWrite(#[from] TurnWriteError),

    #[error("config error: {message}")]
    Config { message: String },
}

type TrackerRegistry = HashMap<String, HashMap<SessionKey, Instant>>;

static IDLE_TRACKERS: OnceLock<Mutex<TrackerRegistry>> = OnceLock::new();

pub fn record_turn(db_path: &str, namespace: Option<&str>, session_id: &str) {
    record_turn_at(db_path, namespace, session_id, Instant::now());
}

pub fn record_turn_at(db_path: &str, namespace: Option<&str>, session_id: &str, seen_at: Instant) {
    let mut trackers = tracker_registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    trackers
        .entry(db_path.to_owned())
        .or_default()
        .insert(session_key(namespace, session_id), seen_at);
}

pub fn clear_session(db_path: &str, namespace: Option<&str>, session_id: &str) {
    let mut trackers = tracker_registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let key = session_key(namespace, session_id);
    let remove_db_entry = if let Some(sessions) = trackers.get_mut(db_path) {
        sessions.remove(&key);
        sessions.is_empty()
    } else {
        false
    };
    if remove_db_entry {
        trackers.remove(db_path);
    }
}

pub fn is_idle_at(
    db_path: &str,
    namespace: Option<&str>,
    session_id: &str,
    now: Instant,
    idle_for: Duration,
) -> bool {
    let trackers = tracker_registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    trackers
        .get(db_path)
        .and_then(|sessions| sessions.get(&session_key(namespace, session_id)).copied())
        .and_then(|seen_at| now.checked_duration_since(seen_at))
        .map(|elapsed| elapsed >= idle_for)
        .unwrap_or(false)
}

pub fn scan_due_sessions(
    db: &Connection,
    db_path: &str,
) -> Result<Vec<IdleCloseResult>, IdleCloseError> {
    scan_due_sessions_at(db, db_path, Instant::now())
}

pub fn scan_due_sessions_at(
    db: &Connection,
    db_path: &str,
    now: Instant,
) -> Result<Vec<IdleCloseResult>, IdleCloseError> {
    let idle_for = idle_close_duration(db)?;
    let due_sessions = due_sessions_at(db_path, now, idle_for);
    let mut closed_sessions = Vec::new();

    for key in due_sessions {
        let close_result = match turn_writer::close_session_if_idle(
            db,
            &key.session_id,
            key.namespace.as_deref(),
            idle_for,
            now,
        ) {
            Ok(Some(result)) => result,
            Ok(None) => continue,
            Err(TurnWriteError::SessionNotFound { .. }) => {
                clear_session(db_path, key.namespace.as_deref(), &key.session_id);
                continue;
            }
            Err(error) => return Err(error.into()),
        };

        let scheduled_for = queue::current_timestamp(db)?;
        let queue_session_id = queue::session_queue_key(key.namespace.as_deref(), &key.session_id);
        queue::enqueue(
            db,
            &queue_session_id,
            &close_result.conversation_path,
            ExtractionTriggerKind::SessionClose,
            &scheduled_for,
        )?;
        closed_sessions.push(IdleCloseResult {
            namespace: key.namespace,
            session_id: key.session_id,
            conversation_path: close_result.conversation_path,
            scheduled_for,
            newly_closed: close_result.newly_closed,
        });
    }

    Ok(closed_sessions)
}

fn idle_close_duration(db: &Connection) -> Result<Duration, IdleCloseError> {
    let raw = db::read_config_value_or(
        db,
        "extraction.idle_close_ms",
        &DEFAULT_IDLE_CLOSE_MS.to_string(),
    )
    .map_err(|error| IdleCloseError::Config {
        message: error.to_string(),
    })?;
    let idle_close_ms = raw.parse::<u64>().map_err(|_| IdleCloseError::Config {
        message: format!("invalid extraction.idle_close_ms value: {raw}"),
    })?;
    Ok(Duration::from_millis(idle_close_ms))
}

fn due_sessions_at(db_path: &str, now: Instant, idle_for: Duration) -> Vec<SessionKey> {
    let trackers = tracker_registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    trackers
        .get(db_path)
        .into_iter()
        .flat_map(|sessions| sessions.iter())
        .filter_map(|(key, seen_at)| {
            now.checked_duration_since(*seen_at)
                .filter(|elapsed| *elapsed >= idle_for)
                .map(|_| key.clone())
        })
        .collect()
}

fn session_key(namespace: Option<&str>, session_id: &str) -> SessionKey {
    SessionKey {
        namespace: namespace
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        session_id: session_id.to_owned(),
    }
}

fn tracker_registry() -> &'static Mutex<TrackerRegistry> {
    IDLE_TRACKERS.get_or_init(|| Mutex::new(HashMap::new()))
}
