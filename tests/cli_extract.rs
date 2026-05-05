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
fn extract_single_session_force_resets_all_day_file_cursors_before_enqueue() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("memory.db");
    let conn = open_test_db(&db_path);
    let vault_root = dir.path().join("vault");
    write_conversation_file(&vault_root, "conversations/2026-05-04/s1.md", "s1", 4);
    write_conversation_file(&vault_root, "conversations/2026-05-05/s1.md", "s1", 2);
    drop(conn);

    let output = run_quaid(&db_path, &["extract", "s1", "--force"]);
    assert!(
        output.status.success(),
        "extract --force failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let older = format::parse(
        &vault_root
            .join("conversations")
            .join("2026-05-04")
            .join("s1.md"),
    )
    .unwrap();
    let newer = format::parse(
        &vault_root
            .join("conversations")
            .join("2026-05-05")
            .join("s1.md"),
    )
    .unwrap();
    assert_eq!(older.frontmatter.last_extracted_turn, 0);
    assert_eq!(older.frontmatter.last_extracted_at, None);
    assert_eq!(newer.frontmatter.last_extracted_turn, 0);
    assert_eq!(newer.frontmatter.last_extracted_at, None);

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
