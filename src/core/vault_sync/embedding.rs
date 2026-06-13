//! Embedding queue worker.
//!
//! `vault_sync` runs two background threads per serve session: the
//! supervisor (heartbeats, watcher polls, restore safety) and the
//! extraction worker. The extraction worker drives the conversation
//! extractor; this module owns the *embedding* lane that converts
//! ready pages into `page_embeddings_vec_*` rows.
//!
//! [`drain_embedding_queue`] is the single public entry the
//! supervisor's drain interval calls every
//! [`drain_interval_secs`] seconds; it claims a bounded number of
//! `embedding_jobs` rows and processes them either inline (for
//! `:memory:` databases or single-claim batches) or fanned out onto
//! short-lived worker threads (for file-backed databases). Each
//! worker opens its own connection and applies
//! `crate::core::inference::refresh_page_embeddings` to the page,
//! then deletes the job row on success or stamps `last_error` and
//! flips it back to `failed` on error.
//!
//! [`resume_orphaned_embedding_jobs`] is the startup recovery hook:
//! any rows left in `running` state from a previous serve process
//! are re-armed as `pending` so the next drain pass picks them up.
//!
//! [`run_extraction_worker`] is the conversation-extractor lane —
//! it owns its own loop and connection, and is unrelated to the
//! embedding queue beyond sharing the same supervisor's stop
//! signal. It lives here because it is a sibling background worker
//! the supervisor spawns alongside the embedding lane.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

use crate::commands::get::get_page_by_key;
use crate::core::conversation::extractor::Worker;
use crate::core::conversation::slm::LazySlmRunner;
use crate::core::conversation::supersede::ResolvingFactWriter;

use super::{database_path, VaultSyncError};

/// Cadence the supervisor uses between `drain_embedding_queue`
/// passes. Held at 2s so a freshly written page is reflected in
/// vector search within a few seconds, matching the watcher's
/// debounce window.
pub(super) fn drain_interval_secs() -> u64 {
    2
}

/// Resets any `running` embedding jobs left over from a previous
/// serve process back to `pending` so the next drain claims them.
/// Called at the start of `start_serve_runtime`.
pub(crate) fn resume_orphaned_embedding_jobs(conn: &Connection) -> Result<usize, VaultSyncError> {
    conn.execute(
        "UPDATE embedding_jobs
         SET job_state = 'pending',
             started_at = NULL
         WHERE job_state = 'running'",
        [],
    )
    .map_err(Into::into)
}

/// Conversation-extractor background worker. Loops until the
/// supervisor's stop signal is set, sleeping a second between
/// failed `run_once` calls so a transient extractor error does not
/// hot-loop the CPU.
pub(super) fn run_extraction_worker(db_path: String, stop: Arc<AtomicBool>, session_id: String) {
    // Route through `db::open_runtime` so the worker inherits the 5s
    // busy_timeout (and WAL/sqlite-vec setup) instead of a bare connection
    // that fails fast under write contention (review items #7 + #10).
    let conn = match crate::core::db::open_runtime(&db_path) {
        Ok(conn) => conn,
        Err(error) => {
            eprintln!(
                "WARN: extraction_worker_db_open_failed session_id={} error={}",
                session_id, error
            );
            return;
        }
    };
    let worker = match Worker::new(&conn, LazySlmRunner::new(), ResolvingFactWriter) {
        Ok(worker) => worker,
        Err(error) => {
            eprintln!(
                "WARN: extraction_worker_init_failed session_id={} error={}",
                session_id, error
            );
            return;
        }
    };
    while !stop.load(Ordering::SeqCst) {
        if let Err(error) = worker.run_once() {
            eprintln!(
                "WARN: extraction_worker_run_failed session_id={} error={}",
                session_id, error
            );
            thread::sleep(Duration::from_secs(1));
        }
    }
}

/// Drives one drain-pass over the embedding queue. Claims up to
/// `configured_concurrency()` ready jobs in a single transaction,
/// then processes them either inline (`:memory:` / single-claim) or
/// in parallel on short-lived worker threads.
pub fn drain_embedding_queue(conn: &Connection) -> Result<usize, VaultSyncError> {
    let claimed = claim_embedding_jobs(conn)?;
    if claimed.is_empty() {
        return Ok(0);
    }

    let db_path = database_path(conn).unwrap_or_default();
    if db_path.is_empty() || db_path == ":memory:" || claimed.len() == 1 {
        let mut processed = 0usize;
        for job in claimed {
            match process_embedding_job_on_connection(conn, job.id, job.page_id) {
                Ok(()) => processed += 1,
                Err(error) => {
                    mark_embedding_job_failed(conn, job.id, &error)?;
                    if job.attempt_count >= 5 {
                        eprintln!(
                            "WARN: embedding_job_failed_permanently job_id={} page_id={} error={}",
                            job.id, job.page_id, error
                        );
                    }
                }
            }
        }
        return Ok(processed);
    }

    let mut handles = Vec::new();
    for job in claimed {
        let path = db_path.clone();
        handles.push(thread::spawn(move || {
            let conn = crate::core::db::open(&path).map_err(|error| {
                VaultSyncError::InvariantViolation {
                    message: format!("embedding worker failed to open database: {error}"),
                }
            })?;
            let result = process_embedding_job_on_connection(&conn, job.id, job.page_id);
            if let Err(ref error) = result {
                let _ = mark_embedding_job_failed(&conn, job.id, error);
            }
            result.map(|_| (job.id, job.page_id, job.attempt_count))
        }));
    }

    let mut processed = 0usize;
    for handle in handles {
        match handle.join() {
            Ok(Ok(_)) => processed += 1,
            Ok(Err(error)) => {
                eprintln!("WARN: embedding_job_failed error={error}");
            }
            Err(_) => {
                eprintln!("WARN: embedding_worker_thread_panicked");
            }
        }
    }

    Ok(processed)
}

#[derive(Debug, Clone)]
struct EmbeddingJobClaim {
    id: i64,
    page_id: i64,
    attempt_count: i64,
}

/// (id, page_id, job_state, attempt_count, last_attempt_epoch)
type EmbeddingJobRow = (i64, i64, String, i64, i64);

fn configured_concurrency() -> usize {
    std::env::var("QUAID_EMBEDDING_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1)
                .min(4)
        })
}

fn backoff_secs(attempt_count: i64) -> i64 {
    match attempt_count {
        count if count <= 0 => 0,
        count => 1_i64 << ((count - 1).min(4) as u32),
    }
}

fn load_job_candidates(conn: &Connection) -> Result<Vec<EmbeddingJobRow>, VaultSyncError> {
    let mut stmt = conn.prepare(
        "SELECT id,
                page_id,
                job_state,
                attempt_count,
                COALESCE(
                    CAST(strftime('%s', started_at) AS INTEGER),
                    CAST(strftime('%s', enqueued_at) AS INTEGER),
                    0
                ) AS last_attempt_epoch
         FROM embedding_jobs
         WHERE job_state IN ('pending', 'failed')
           AND attempt_count < 5
         ORDER BY priority DESC, enqueued_at ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn claim_embedding_jobs(conn: &Connection) -> Result<Vec<EmbeddingJobClaim>, VaultSyncError> {
    let now_epoch = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default();
    let limit = configured_concurrency();
    let candidates = load_job_candidates(conn)?;
    let ready = candidates
        .into_iter()
        .filter(|(_, _, state, attempt_count, last_attempt_epoch)| {
            state == "pending" || now_epoch - *last_attempt_epoch >= backoff_secs(*attempt_count)
        })
        .take(limit)
        .collect::<Vec<_>>();

    if ready.is_empty() {
        return Ok(Vec::new());
    }

    let tx = conn.unchecked_transaction()?;
    let mut claimed = Vec::new();
    for (id, page_id, _state, _attempt_count, _last_attempt_epoch) in ready {
        let updated = tx.execute(
            "UPDATE embedding_jobs
             SET job_state = 'running',
                 started_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                 attempt_count = attempt_count + 1,
                 last_error = NULL
             WHERE id = ?1
               AND job_state IN ('pending', 'failed')
               AND attempt_count < 5",
            [id],
        )?;
        if updated == 1 {
            let attempt_count = tx.query_row(
                "SELECT attempt_count FROM embedding_jobs WHERE id = ?1",
                [id],
                |row| row.get(0),
            )?;
            claimed.push(EmbeddingJobClaim {
                id,
                page_id,
                attempt_count,
            });
        }
    }
    tx.commit()?;
    Ok(claimed)
}

fn load_page_for_embedding_job(
    conn: &Connection,
    page_id: i64,
) -> Result<Option<crate::core::types::Page>, VaultSyncError> {
    let row = conn
        .query_row(
            "SELECT collection_id, slug FROM pages WHERE id = ?1",
            [page_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((collection_id, slug)) = row else {
        return Ok(None);
    };
    get_page_by_key(conn, collection_id, &slug)
        .map(Some)
        .map_err(|error| VaultSyncError::InvariantViolation {
            message: error.to_string(),
        })
}

pub(super) fn process_embedding_job_on_connection(
    conn: &Connection,
    job_id: i64,
    page_id: i64,
) -> Result<(), VaultSyncError> {
    let Some(page) = load_page_for_embedding_job(conn, page_id)? else {
        conn.execute("DELETE FROM embedding_jobs WHERE id = ?1", [job_id])?;
        return Ok(());
    };

    crate::core::inference::refresh_page_embeddings(conn, page_id, &page).map_err(|err| {
        VaultSyncError::InvariantViolation {
            message: format!("embedding refresh failed for page_id={page_id}: {err}"),
        }
    })?;
    conn.execute("DELETE FROM embedding_jobs WHERE id = ?1", [job_id])?;
    Ok(())
}

fn mark_embedding_job_failed(
    conn: &Connection,
    job_id: i64,
    error: &VaultSyncError,
) -> Result<(), VaultSyncError> {
    conn.execute(
        "UPDATE embedding_jobs
         SET job_state = 'failed',
             last_error = ?2
         WHERE id = ?1",
        params![job_id, error.to_string()],
    )?;
    Ok(())
}

#[cfg(test)]
pub(super) fn configured_concurrency_for_test() -> usize {
    configured_concurrency()
}
