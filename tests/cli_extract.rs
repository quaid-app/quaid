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
use rusqlite::Connection;

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
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    command.arg("--db").arg(db_path).args(args);
    command.output().expect("run quaid")
}

fn write_conversation_file(
    root: &Path,
    relative_path: &str,
    session_id: &str,
    last_extracted_turn: i64,
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
            started_at: "2026-05-05T09:00:00Z".to_string(),
            status: ConversationStatus::Open,
            closed_at: None,
            last_extracted_at: Some("2026-05-05T10:00:00Z".to_string()),
            last_extracted_turn,
        },
        turns: vec![Turn {
            ordinal: last_extracted_turn.max(1),
            role: TurnRole::User,
            timestamp: "2026-05-05T09:00:00Z".to_string(),
            content: format!("turn for {session_id}"),
            metadata: None,
        }],
    };
    fs::write(absolute, format::render(&file)).expect("write conversation");
}

fn queue_rows(conn: &Connection) -> Vec<(String, String, String, String)> {
    let mut stmt = conn
        .prepare(
            "SELECT session_id, conversation_path, trigger_kind, status
             FROM extraction_queue
             ORDER BY session_id",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .unwrap();
    rows.map(|row| row.unwrap()).collect()
}

#[test]
fn extract_single_session_enqueues_manual_job() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_test_db(&db_path);
    let vault_root = dir.path().join("vault");
    write_conversation_file(&vault_root, "conversations/2026-05-05/s1.md", "s1", 3);
    drop(conn);

    let output = run_quaid(&db_path, &["extract", "s1"]);
    assert!(
        output.status.success(),
        "extract failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Enqueued manual extraction"));
    assert!(stdout.contains("s1"));
    assert!(stdout.contains("quaid extraction status"));

    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    assert_eq!(
        queue_rows(&conn),
        vec![(
            "s1".to_string(),
            "conversations/2026-05-05/s1.md".to_string(),
            "manual".to_string(),
            "pending".to_string()
        )]
    );

    // Bare extract must not touch the day-file cursor.
    let day_file = format::parse(
        &vault_root
            .join("conversations")
            .join("2026-05-05")
            .join("s1.md"),
    )
    .unwrap();
    assert_eq!(
        day_file.frontmatter.last_extracted_turn, 3,
        "bare extract must not reset last_extracted_turn"
    );
}

#[test]
fn extract_single_session_force_resets_all_day_file_cursors_and_enqueues_one_job_per_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_test_db(&db_path);
    let vault_root = dir.path().join("vault");
    write_conversation_file(&vault_root, "conversations/2026-05-03/s1.md", "s1", 7);
    write_conversation_file(&vault_root, "conversations/2026-05-04/s1.md", "s1", 4);
    write_conversation_file(&vault_root, "conversations/2026-05-05/s1.md", "s1", 2);
    drop(conn);

    let output = run_quaid(&db_path, &["extract", "s1", "--force"]);
    assert!(
        output.status.success(),
        "extract --force failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    for date in ["2026-05-03", "2026-05-04", "2026-05-05"] {
        let parsed = format::parse(&vault_root.join("conversations").join(date).join("s1.md"))
            .unwrap_or_else(|err| panic!("parse {date}: {err}"));
        assert_eq!(
            parsed.frontmatter.last_extracted_turn, 0,
            "{date}: cursor must be reset to 0"
        );
        assert_eq!(
            parsed.frontmatter.last_extracted_at, None,
            "{date}: last_extracted_at must be cleared"
        );
    }

    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    assert_eq!(
        queue_rows(&conn),
        vec![
            (
                "s1".to_string(),
                "conversations/2026-05-03/s1.md".to_string(),
                "manual".to_string(),
                "pending".to_string(),
            ),
            (
                "s1".to_string(),
                "conversations/2026-05-04/s1.md".to_string(),
                "manual".to_string(),
                "pending".to_string(),
            ),
            (
                "s1".to_string(),
                "conversations/2026-05-05/s1.md".to_string(),
                "manual".to_string(),
                "pending".to_string(),
            ),
        ],
        "force re-extract must enqueue one manual job per day-file (chronological)",
    );
}

#[test]
fn extract_force_is_idempotent_and_does_not_grow_the_queue() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_test_db(&db_path);
    let vault_root = dir.path().join("vault");
    write_conversation_file(&vault_root, "conversations/2026-05-04/s1.md", "s1", 4);
    write_conversation_file(&vault_root, "conversations/2026-05-05/s1.md", "s1", 2);
    drop(conn);

    for _ in 0..3 {
        let output = run_quaid(&db_path, &["extract", "s1", "--force"]);
        assert!(
            output.status.success(),
            "extract --force failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    let rows = queue_rows(&conn);
    assert_eq!(
        rows.len(),
        2,
        "repeated --force must collapse per (session, path), got {rows:?}"
    );
}

#[test]
fn extract_all_enqueues_every_known_session() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_test_db(&db_path);
    let vault_root = dir.path().join("vault");
    write_conversation_file(&vault_root, "conversations/2026-05-04/s1.md", "s1", 1);
    write_conversation_file(&vault_root, "conversations/2026-05-05/s2.md", "s2", 1);
    drop(conn);

    let output = run_quaid(&db_path, &["extract", "--all"]);
    assert!(
        output.status.success(),
        "extract --all failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("s1"));
    assert!(stdout.contains("s2"));
    assert!(stdout.contains("quaid extraction status"));

    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    assert_eq!(
        queue_rows(&conn),
        vec![
            (
                "s1".to_string(),
                "conversations/2026-05-04/s1.md".to_string(),
                "manual".to_string(),
                "pending".to_string()
            ),
            (
                "s2".to_string(),
                "conversations/2026-05-05/s2.md".to_string(),
                "manual".to_string(),
                "pending".to_string()
            ),
        ]
    );
}

#[test]
fn extract_all_since_filters_to_sessions_with_matching_day_files() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_test_db(&db_path);
    let vault_root = dir.path().join("vault");
    write_conversation_file(&vault_root, "conversations/2026-04-30/april.md", "april", 1);
    write_conversation_file(&vault_root, "conversations/2026-05-02/may.md", "may", 1);
    write_conversation_file(&vault_root, "conversations/2026-04-29/mixed.md", "mixed", 1);
    write_conversation_file(&vault_root, "conversations/2026-05-01/mixed.md", "mixed", 1);
    drop(conn);

    let output = run_quaid(&db_path, &["extract", "--all", "--since", "2026-05-01"]);
    assert!(
        output.status.success(),
        "extract --all --since failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    assert_eq!(
        queue_rows(&conn),
        vec![
            (
                "may".to_string(),
                "conversations/2026-05-02/may.md".to_string(),
                "manual".to_string(),
                "pending".to_string()
            ),
            (
                "mixed".to_string(),
                "conversations/2026-05-01/mixed.md".to_string(),
                "manual".to_string(),
                "pending".to_string()
            ),
        ]
    );
}

#[cfg(unix)]
#[test]
fn extract_force_blocks_while_another_process_holds_the_session_file_lock() {
    use std::os::fd::AsRawFd;
    use std::time::{Duration, Instant};

    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_test_db(&db_path);
    let vault_root = dir.path().join("vault");
    write_conversation_file(&vault_root, "conversations/2026-05-05/s1.md", "s1", 0);
    drop(conn);

    // Acquire the on-disk SessionFileLock that turn_writer::append_turn (and the
    // new with_session_locks helper) take before mutating any day-file. If the
    // CLI's reset_cursors path does not contend on this lock, the assertion that
    // it waited for ~hold_ms below will fail.
    let lock_path = vault_root
        .join("conversations")
        .join(".locks")
        .join("s1.lock");
    fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    let rc = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX) };
    assert_eq!(rc, 0, "test failed to acquire LOCK_EX");

    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    command
        .arg("--db")
        .arg(&db_path)
        .args(["extract", "s1", "--force"]);
    let mut child = command.spawn().expect("spawn quaid extract --force");

    let hold_ms = 500u64;
    std::thread::sleep(Duration::from_millis(hold_ms));
    // Child must still be running while we hold the lock.
    assert!(
        child.try_wait().unwrap().is_none(),
        "extract --force must block on the session lock while another writer holds it"
    );

    let started_release = Instant::now();
    let rc = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_UN) };
    assert_eq!(rc, 0, "test failed to release LOCK_UN");
    drop(lock_file);

    let status = child.wait().expect("await quaid extract --force");
    assert!(
        status.success(),
        "extract --force failed after lock released ({:?})",
        status
    );
    assert!(
        started_release.elapsed() < Duration::from_secs(10),
        "extract --force took unreasonably long after lock released"
    );

    let parsed = format::parse(
        &vault_root
            .join("conversations")
            .join("2026-05-05")
            .join("s1.md"),
    )
    .unwrap();
    assert_eq!(parsed.frontmatter.last_extracted_turn, 0);
}

#[test]
fn extract_force_requires_session_id() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_test_db(&db_path);
    drop(conn);

    let output = run_quaid(&db_path, &["extract", "--force"]);
    assert!(
        !output.status.success(),
        "--force without session_id should fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("session_id")
            || stderr.contains("SESSION_ID")
            || stderr.contains("required"),
        "expected clap error about missing session_id, got: {stderr}"
    );
}
