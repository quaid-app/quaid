#![expect(
    clippy::print_stdout,
    reason = "CLI command prints user-facing output to stdout by design"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Subcommand;
use rusqlite::{params, Connection};

use crate::core::{
    conversation::model_lifecycle::{
        cached_model_status, download_model, load_model_from_local_cache, ConsoleProgressReporter,
    },
    conversation::{format, turn_writer},
    db,
    types::ConversationStatus,
};

#[derive(Subcommand)]
pub enum ExtractionAction {
    /// Enable SLM extraction and eagerly cache the configured model
    Enable,
    /// Disable SLM extraction without deleting cached model files
    Disable,
    /// Show current extraction configuration and cache state
    Status,
}

pub fn run(db: &Connection, action: ExtractionAction) -> Result<()> {
    match action {
        ExtractionAction::Enable => enable(db),
        ExtractionAction::Disable => disable(db),
        ExtractionAction::Status => status(db),
    }
}

fn enable(db: &Connection) -> Result<()> {
    let alias = db::read_config_value_or(db, "extraction.model_alias", "phi-3.5-mini")?;
    let mut progress = ConsoleProgressReporter::default();
    let cache_dir = tokio::task::block_in_place(|| download_model(&alias, &mut progress))?;
    set_extraction_enabled(db, true)?;
    println!("Extraction enabled: yes");
    println!("Model alias: {alias}");
    println!("Model cache: {}", cache_dir.display());
    Ok(())
}

fn disable(db: &Connection) -> Result<()> {
    set_extraction_enabled(db, false)?;
    println!("Extraction enabled: no");
    Ok(())
}

fn status(db: &Connection) -> Result<()> {
    let enabled = extraction_enabled(db)?;
    let alias = db::read_config_value_or(db, "extraction.model_alias", "phi-3.5-mini")?;
    let cache_status = cached_model_status(&alias)?;
    let counts = queue_counts(db)?;
    let idle_close_ms = parse_i64_config(db, "extraction.idle_close_ms", 60_000)?;
    let sessions = session_summaries(db)?;
    let active_sessions = active_sessions(db, &sessions, idle_close_ms)?;
    let failed_jobs = recent_failed_jobs(db)?;
    let cache_state = match (
        cache_status.is_cached,
        cache_status.verified,
        cache_status.source_pinned,
    ) {
        (true, true, true) => "verified (source-pinned)",
        (true, true, false) => "verified (manifest-only)",
        (true, false, _) => "present but invalid",
        (false, _, _) => "missing",
    };
    let runtime_state = runtime_state(enabled, &alias, &sessions);

    println!("Extraction enabled: {}", yes_no(enabled));
    println!("Model alias: {}", cache_status.alias.requested_alias);
    println!("Model repo: {}", cache_status.alias.repo_id);
    println!("Model cache dir: {}", cache_status.cache_dir.display());
    println!("Model cache state: {cache_state}");
    println!("Runtime state: {runtime_state}");
    println!(
        "Estimated resident memory when loaded: {}",
        estimated_resident_memory(&cache_status.alias.requested_alias)
    );
    println!(
        "Queue depth: pending={} running={} failed_last_24h={}",
        counts.pending, counts.running, counts.failed_recent
    );
    print_active_sessions(idle_close_ms, &active_sessions);
    print_failed_jobs(&failed_jobs);
    Ok(())
}

fn queue_counts(db: &Connection) -> Result<QueueCounts> {
    let counts = db.query_row(
        "SELECT
             COALESCE(SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE
                 WHEN status = 'failed'
                  AND julianday(enqueued_at) >= julianday('now', '-1 day')
                 THEN 1
                 ELSE 0
             END), 0)
         FROM extraction_queue",
        [],
        |row| {
            Ok(QueueCounts {
                pending: row.get(0)?,
                running: row.get(1)?,
                failed_recent: row.get(2)?,
            })
        },
    )?;
    Ok(counts)
}

fn recent_failed_jobs(db: &Connection) -> Result<Vec<FailedJobStatus>> {
    let mut stmt = db.prepare(
        "SELECT session_id, attempts, COALESCE(last_error, '')
         FROM extraction_queue
         WHERE status = 'failed'
           AND julianday(enqueued_at) >= julianday('now', '-1 day')
         ORDER BY scheduled_for DESC, id DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(FailedJobStatus {
            session_id: row.get(0)?,
            attempts: row.get(1)?,
            last_error: truncate_chars(&row.get::<_, String>(2)?, 200),
        })
    })?;
    let mut failed = Vec::new();
    for row in rows {
        failed.push(row?);
    }
    Ok(failed)
}

fn session_summaries(db: &Connection) -> Result<Vec<SessionSummary>> {
    let root = turn_writer::resolve_memory_root(db)?;
    let mut sessions = BTreeMap::<String, SessionSummary>::new();
    for relative_path in conversation_paths(&root.root_path)? {
        let conversation_path = root
            .root_path
            .join(relative_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        let parsed_path = format::parse_relative_conversation_path(&relative_path)?;
        let conversation = format::parse(&conversation_path)?;
        let last_turn_at = conversation
            .turns
            .last()
            .map(|turn| turn.timestamp.clone())
            .unwrap_or_else(|| conversation.frontmatter.started_at.clone());
        let key = session_key(parsed_path.namespace.as_deref(), &parsed_path.session_id);
        let summary = SessionSummary {
            display_name: session_display_name(
                parsed_path.namespace.as_deref(),
                &parsed_path.session_id,
            ),
            last_turn_at,
            last_extracted_at: conversation.frontmatter.last_extracted_at.clone(),
            status: conversation.frontmatter.status,
        };
        merge_session_summary(
            sessions.entry(key).or_insert_with(|| summary.clone()),
            summary,
        );
    }
    Ok(sessions.into_values().collect())
}

fn active_sessions(
    db: &Connection,
    sessions: &[SessionSummary],
    idle_close_ms: i64,
) -> Result<Vec<ActiveSessionStatus>> {
    let mut active = Vec::new();
    for session in sessions {
        if session.status != ConversationStatus::Open {
            continue;
        }
        let idle_seconds = seconds_since(db, &session.last_turn_at)?;
        if idle_seconds.saturating_mul(1000) > idle_close_ms {
            continue;
        }
        active.push(ActiveSessionStatus {
            display_name: session.display_name.clone(),
            idle_seconds,
            last_extracted_at: session.last_extracted_at.clone(),
        });
    }
    active.sort_by(|left, right| {
        left.idle_seconds
            .cmp(&right.idle_seconds)
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
    Ok(active)
}

fn conversation_paths(root: &Path) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    for base in conversation_roots(root)? {
        let conversations_dir = root.join(&base);
        if !conversations_dir.is_dir() {
            continue;
        }
        for date_entry in fs::read_dir(&conversations_dir)? {
            let date_entry = date_entry?;
            let date_path = date_entry.path();
            if !date_path.is_dir() {
                continue;
            }
            for file_entry in fs::read_dir(&date_path)? {
                let file_entry = file_entry?;
                let file_path = file_entry.path();
                if !file_path.is_file()
                    || file_path.extension().and_then(|ext| ext.to_str()) != Some("md")
                {
                    continue;
                }
                let relative = file_path
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                paths.push(relative);
            }
        }
    }
    paths.sort();
    Ok(paths)
}

fn conversation_roots(root: &Path) -> Result<Vec<PathBuf>> {
    let mut roots = vec![PathBuf::from("conversations")];
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(namespace) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if namespace == "conversations" {
            continue;
        }
        let relative = PathBuf::from(namespace).join("conversations");
        if root.join(&relative).is_dir() {
            roots.push(relative);
        }
    }
    roots.sort();
    Ok(roots)
}

fn runtime_state(enabled: bool, alias: &str, sessions: &[SessionSummary]) -> String {
    if !enabled {
        return "disabled".to_string();
    }
    match load_model_from_local_cache(alias) {
        Ok(path) => {
            if sessions
                .iter()
                .any(|session| session.last_extracted_at.is_some())
            {
                format!(
                    "local cache ready at {} (prior extraction recorded)",
                    path.display()
                )
            } else {
                "not loaded yet".to_string()
            }
        }
        Err(error) => format!("blocked ({error})"),
    }
}

fn estimated_resident_memory(alias: &str) -> &'static str {
    match alias.trim().to_ascii_lowercase().as_str() {
        "phi-3.5-mini" => "~2.0 GiB",
        "gemma-3-1b" => "~600 MiB",
        "gemma-3-4b" => "~2.0 GiB+",
        _ => "unknown",
    }
}

fn print_active_sessions(idle_close_ms: i64, sessions: &[ActiveSessionStatus]) {
    if sessions.is_empty() {
        println!(
            "Active sessions (idle window {}): none",
            human_duration_ms(idle_close_ms)
        );
        return;
    }
    println!(
        "Active sessions (idle window {}):",
        human_duration_ms(idle_close_ms)
    );
    for session in sessions {
        println!(
            "  - {} — idle {} — last extracted: {}",
            session.display_name,
            human_duration_seconds(session.idle_seconds),
            session.last_extracted_at.as_deref().unwrap_or("never")
        );
    }
}

fn print_failed_jobs(failed_jobs: &[FailedJobStatus]) {
    if failed_jobs.is_empty() {
        println!("Failed jobs (last 24h): none");
        return;
    }
    println!("Failed jobs (last 24h):");
    for job in failed_jobs {
        println!(
            "  - {} — attempts: {} — {}",
            job.session_id, job.attempts, job.last_error
        );
    }
    println!(
        "  Rerun with `quaid extract <session> --force`; if failures persist, try another `extraction.model_alias`."
    );
}

fn seconds_since(db: &Connection, timestamp: &str) -> Result<i64> {
    db.query_row(
        "SELECT CAST(MAX(0, (julianday('now') - julianday(?1)) * 86400) AS INTEGER)",
        [timestamp],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn parse_i64_config(db: &Connection, key: &str, default: i64) -> Result<i64> {
    let raw = db::read_config_value_or(db, key, &default.to_string())?;
    raw.parse::<i64>()
        .map_err(|_| anyhow::anyhow!("invalid {key} value: {raw}"))
}

fn session_key(namespace: Option<&str>, session_id: &str) -> String {
    match namespace.filter(|value| !value.is_empty()) {
        Some(namespace) => format!("{namespace}::{session_id}"),
        None => session_id.to_string(),
    }
}

fn session_display_name(namespace: Option<&str>, session_id: &str) -> String {
    match namespace.filter(|value| !value.is_empty()) {
        Some(namespace) => format!("{namespace}/{session_id}"),
        None => session_id.to_string(),
    }
}

fn merge_session_summary(target: &mut SessionSummary, candidate: SessionSummary) {
    if candidate.last_turn_at >= target.last_turn_at {
        target.last_turn_at = candidate.last_turn_at;
        target.status = candidate.status;
    }
    if candidate.last_extracted_at.as_deref().unwrap_or("")
        >= target.last_extracted_at.as_deref().unwrap_or("")
    {
        target.last_extracted_at = candidate.last_extracted_at;
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let truncated = value.chars().take(max_chars).collect::<String>();
    format!("{truncated}…")
}

fn human_duration_ms(milliseconds: i64) -> String {
    human_duration_seconds(milliseconds.div_euclid(1000))
}

fn human_duration_seconds(seconds: i64) -> String {
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    let remainder = seconds % 60;
    if remainder == 0 {
        format!("{minutes}m")
    } else {
        format!("{minutes}m{remainder}s")
    }
}

fn extraction_enabled(db: &Connection) -> Result<bool> {
    Ok(matches!(
        db::read_config_value_or(db, "extraction.enabled", "false")?.as_str(),
        "true"
    ))
}

fn set_extraction_enabled(db: &Connection, enabled: bool) -> Result<()> {
    db.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES ('extraction.enabled', ?1)",
        params![if enabled { "true" } else { "false" }],
    )?;
    Ok(())
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

#[derive(Debug)]
struct QueueCounts {
    pending: i64,
    running: i64,
    failed_recent: i64,
}

#[derive(Clone, Debug)]
struct SessionSummary {
    display_name: String,
    last_turn_at: String,
    last_extracted_at: Option<String>,
    status: ConversationStatus,
}

#[derive(Debug)]
struct ActiveSessionStatus {
    display_name: String,
    idle_seconds: i64,
    last_extracted_at: Option<String>,
}

#[derive(Debug)]
struct FailedJobStatus {
    session_id: String,
    attempts: i64,
    last_error: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::TurnRole;

    fn configured_db() -> (tempfile::TempDir, Connection, PathBuf) {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let db_path = dir.path().join("memory.db");
        let conn = db::open(db_path.to_str().expect("utf8 db path")).expect("open db");
        let vault_root = dir.path().join("vault");
        fs::create_dir_all(&vault_root).expect("vault root");
        conn.execute(
            "UPDATE collections
             SET root_path = ?1,
                 state = 'active',
                 writable = 1,
                 is_write_target = 1
             WHERE id = 1",
            [vault_root.display().to_string()],
        )
        .expect("configure root");
        (dir, conn, vault_root)
    }

    #[test]
    fn queue_counts_and_recent_failed_jobs_report_status_breakdown() {
        let (_dir, conn, _root) = configured_db();
        conn.execute_batch(
            "INSERT INTO extraction_queue
                 (session_id, conversation_path, trigger_kind, enqueued_at, scheduled_for, attempts, last_error, status)
             VALUES
                 ('pending', 'conversations/2026-05-03/pending.md', 'debounce', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), 0, NULL, 'pending'),
                 ('running', 'conversations/2026-05-03/running.md', 'debounce', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), 1, NULL, 'running'),
                 ('failed', 'conversations/2026-05-03/failed.md', 'debounce', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), 2, 'abcdefghijklmnopqrstuvwxyz', 'failed')",
        )
        .expect("seed queue");

        let counts = queue_counts(&conn).expect("queue counts");
        let failed = recent_failed_jobs(&conn).expect("failed jobs");

        assert_eq!(counts.pending, 1);
        assert_eq!(counts.running, 1);
        assert_eq!(counts.failed_recent, 1);
        assert_eq!(failed[0].session_id, "failed");
        assert_eq!(failed[0].attempts, 2);
    }

    #[test]
    fn session_summaries_merge_namespaced_and_daily_conversations() {
        let (_dir, conn, _root) = configured_db();
        turn_writer::append_turn(
            &conn,
            "session-a",
            TurnRole::User,
            "first",
            "2026-05-03T09:14:22Z",
            None,
            None,
        )
        .expect("append first");
        turn_writer::append_turn(
            &conn,
            "session-a",
            TurnRole::Assistant,
            "second",
            "2026-05-04T09:14:22Z",
            None,
            Some("alpha"),
        )
        .expect("append namespaced");

        let summaries = session_summaries(&conn).expect("session summaries");
        let paths = conversation_paths(&turn_writer::resolve_memory_root(&conn).unwrap().root_path)
            .expect("conversation paths");

        assert!(summaries
            .iter()
            .any(|summary| summary.display_name == "session-a"));
        assert!(summaries
            .iter()
            .any(|summary| summary.display_name == "alpha/session-a"));
        assert_eq!(
            paths,
            vec![
                "alpha/conversations/2026-05-04/session-a.md".to_owned(),
                "conversations/2026-05-03/session-a.md".to_owned(),
            ]
        );
    }

    #[test]
    fn active_sessions_filters_closed_and_idle_sessions() {
        let (_dir, conn, _root) = configured_db();
        let sessions = vec![
            SessionSummary {
                display_name: "recent".to_owned(),
                last_turn_at: "2999-01-01T00:00:00Z".to_owned(),
                last_extracted_at: None,
                status: ConversationStatus::Open,
            },
            SessionSummary {
                display_name: "closed".to_owned(),
                last_turn_at: "2999-01-01T00:00:00Z".to_owned(),
                last_extracted_at: Some("2026-05-03T00:00:00Z".to_owned()),
                status: ConversationStatus::Closed,
            },
            SessionSummary {
                display_name: "idle".to_owned(),
                last_turn_at: "2000-01-01T00:00:00Z".to_owned(),
                last_extracted_at: None,
                status: ConversationStatus::Open,
            },
        ];

        let active = active_sessions(&conn, &sessions, 60_000).expect("active sessions");

        assert_eq!(active.len(), 1);
        assert_eq!(active[0].display_name, "recent");
    }

    #[test]
    fn helper_formatters_and_config_parsing_are_stable() {
        let (_dir, conn, _root) = configured_db();
        set_extraction_enabled(&conn, true).expect("enable extraction");
        conn.execute(
            "INSERT OR REPLACE INTO config(key, value) VALUES ('test.invalid', 'abc')",
            [],
        )
        .expect("set invalid config");

        assert!(extraction_enabled(&conn).expect("enabled"));
        assert!(parse_i64_config(&conn, "test.invalid", 42).is_err());
        assert_eq!(parse_i64_config(&conn, "test.missing", 42).unwrap(), 42);
        assert_eq!(session_key(Some("alpha"), "s1"), "alpha::s1");
        assert_eq!(session_key(None, "s1"), "s1");
        assert_eq!(session_display_name(Some("alpha"), "s1"), "alpha/s1");
        assert_eq!(session_display_name(None, "s1"), "s1");
        assert_eq!(truncate_chars("abcdef", 3), "abc…");
        assert_eq!(human_duration_ms(90_000), "1m30s");
        assert_eq!(estimated_resident_memory("gemma-3-1b"), "~600 MiB");
        assert_eq!(yes_no(true), "yes");
        assert_eq!(yes_no(false), "no");
    }

    #[test]
    fn status_and_disable_commands_run_without_model_downloads() {
        let (_dir, conn, _root) = configured_db();

        run(&conn, ExtractionAction::Status).expect("status");
        run(&conn, ExtractionAction::Disable).expect("disable");

        assert!(!extraction_enabled(&conn).expect("disabled"));
    }

    #[test]
    fn enabled_status_prints_active_sessions_failed_jobs_and_blocked_runtime() {
        let (_dir, conn, _root) = configured_db();
        set_extraction_enabled(&conn, true).expect("enable extraction");
        turn_writer::append_turn(
            &conn,
            "live-session",
            TurnRole::User,
            "hello",
            "2999-01-01T00:00:00Z",
            None,
            None,
        )
        .expect("append active turn");
        conn.execute_batch(
            "INSERT INTO extraction_queue
                 (session_id, conversation_path, trigger_kind, enqueued_at, scheduled_for, attempts, last_error, status)
             VALUES
                 ('failed-now', 'conversations/2999-01-01/live-session.md', 'manual', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), 3, 'model cache missing', 'failed')",
        )
        .expect("seed failed job");

        run(&conn, ExtractionAction::Status).expect("enabled status");

        let sessions = session_summaries(&conn).expect("sessions");
        assert!(runtime_state(true, "org/missing-model", &sessions).contains("blocked"));
        assert_eq!(active_sessions(&conn, &sessions, 60_000).unwrap().len(), 1);
    }

    #[test]
    fn print_helpers_cover_non_empty_rows_and_short_durations() {
        print_active_sessions(
            30_000,
            &[ActiveSessionStatus {
                display_name: "session".to_owned(),
                idle_seconds: 12,
                last_extracted_at: Some("2026-05-03T00:00:00Z".to_owned()),
            }],
        );
        print_failed_jobs(&[FailedJobStatus {
            session_id: "session".to_owned(),
            attempts: 2,
            last_error: "failed".to_owned(),
        }]);

        assert_eq!(human_duration_seconds(12), "12s");
        assert_eq!(estimated_resident_memory("gemma-3-4b"), "~2.0 GiB+");
        assert_eq!(estimated_resident_memory("custom"), "unknown");
    }
}
