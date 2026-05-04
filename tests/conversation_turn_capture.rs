use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use quaid::core::conversation::format::{parse, parse_str, render};
use quaid::core::conversation::turn_writer::append_turn;
use quaid::core::db;
use quaid::core::types::{ConversationStatus, TurnRole};
use rusqlite::Connection;

fn open_turn_db(root: &Path) -> (tempfile::TempDir, PathBuf, Connection) {
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
    (db_dir, db_path, conn)
}

const APPEND_TURN_HELPER_ENV: &str = "QUAID_TEST_APPEND_TURN_HELPER";
const APPEND_TURN_DB_PATH_ENV: &str = "QUAID_TEST_APPEND_TURN_DB_PATH";
const APPEND_TURN_SESSION_ID_ENV: &str = "QUAID_TEST_APPEND_TURN_SESSION_ID";
const APPEND_TURN_ROLE_ENV: &str = "QUAID_TEST_APPEND_TURN_ROLE";
const APPEND_TURN_CONTENT_ENV: &str = "QUAID_TEST_APPEND_TURN_CONTENT";
const APPEND_TURN_TIMESTAMP_ENV: &str = "QUAID_TEST_APPEND_TURN_TIMESTAMP";

#[test]
fn append_turn_cross_process_helper() {
    if std::env::var_os(APPEND_TURN_HELPER_ENV).is_none() {
        return;
    }

    let db_path = PathBuf::from(std::env::var(APPEND_TURN_DB_PATH_ENV).unwrap());
    let session_id = std::env::var(APPEND_TURN_SESSION_ID_ENV).unwrap();
    let role = std::env::var(APPEND_TURN_ROLE_ENV)
        .unwrap()
        .parse()
        .unwrap();
    let content = std::env::var(APPEND_TURN_CONTENT_ENV).unwrap();
    let timestamp = std::env::var(APPEND_TURN_TIMESTAMP_ENV).unwrap();
    let conn = db::open(db_path.to_str().unwrap()).unwrap();

    append_turn(&conn, &session_id, role, &content, &timestamp, None, None).unwrap();
}

#[test]
fn parse_render_round_trip_preserves_turn_metadata_and_cursor() {
    let rendered = concat!(
        "---\n",
        "type: conversation\n",
        "session_id: session-1\n",
        "date: 2026-05-03\n",
        "started_at: 2026-05-03T09:14:22Z\n",
        "status: open\n",
        "last_extracted_at: null\n",
        "last_extracted_turn: 0\n",
        "---\n\n",
        "## Turn 1 · user · 2026-05-03T09:14:22Z\n\n",
        "hello\n\n",
        "```json turn-metadata\n",
        "{\n",
        "  \"importance\": \"high\",\n",
        "  \"tool_name\": \"bash\"\n",
        "}\n",
        "```\n"
    );

    let parsed = parse_str(rendered).unwrap();

    assert_eq!(render(&parsed), rendered);
    assert_eq!(parsed.frontmatter.last_extracted_turn, 0);
    assert_eq!(
        parsed.turns[0].metadata.as_ref().unwrap()["tool_name"],
        "bash"
    );
}

#[test]
fn append_turn_is_durable_before_return_and_continues_ordinals_across_days() {
    let vault_root = tempfile::TempDir::new().unwrap();
    let (_db_dir, _db_path, conn) = open_turn_db(vault_root.path());

    let first = append_turn(
        &conn,
        "session-1",
        TurnRole::User,
        "first day",
        "2026-05-03T23:59:00Z",
        None,
        None,
    )
    .unwrap();
    let second = append_turn(
        &conn,
        "session-1",
        TurnRole::Assistant,
        "second day",
        "2026-05-04T00:01:00Z",
        Some(serde_json::json!({"tool_name":"bash"})),
        None,
    )
    .unwrap();

    let first_path = vault_root
        .path()
        .join("conversations")
        .join("2026-05-03")
        .join("session-1.md");
    let second_path = vault_root
        .path()
        .join("conversations")
        .join("2026-05-04")
        .join("session-1.md");

    assert_eq!(first.turn_id, "session-1:1");
    assert_eq!(second.turn_id, "session-1:2");
    assert!(fs::read_to_string(&second_path)
        .unwrap()
        .contains("## Turn 2 · assistant · 2026-05-04T00:01:00Z"));

    let parsed_second = parse(&second_path).unwrap();
    assert_eq!(parsed_second.frontmatter.last_extracted_turn, 0);
    assert_eq!(parsed_second.turns[0].ordinal, 2);
    assert!(fs::read_to_string(&first_path)
        .unwrap()
        .contains("first day"));
}

#[test]
fn append_turn_keeps_namespaces_isolated_for_same_session_id() {
    let vault_root = tempfile::TempDir::new().unwrap();
    let (_db_dir, _db_path, conn) = open_turn_db(vault_root.path());

    let alpha = append_turn(
        &conn,
        "main",
        TurnRole::User,
        "alpha turn",
        "2026-05-03T09:14:22Z",
        None,
        Some("alpha"),
    )
    .unwrap();
    let beta = append_turn(
        &conn,
        "main",
        TurnRole::User,
        "beta turn",
        "2026-05-03T09:14:22Z",
        None,
        Some("beta"),
    )
    .unwrap();

    let alpha_path = vault_root
        .path()
        .join("alpha")
        .join("conversations")
        .join("2026-05-03")
        .join("main.md");
    let beta_path = vault_root
        .path()
        .join("beta")
        .join("conversations")
        .join("2026-05-03")
        .join("main.md");

    assert_eq!(
        alpha.conversation_path,
        "alpha/conversations/2026-05-03/main.md"
    );
    assert_eq!(
        beta.conversation_path,
        "beta/conversations/2026-05-03/main.md"
    );
    assert!(fs::read_to_string(alpha_path)
        .unwrap()
        .contains("alpha turn"));
    assert!(!fs::read_to_string(beta_path.clone())
        .unwrap()
        .contains("alpha turn"));
    assert!(fs::read_to_string(beta_path).unwrap().contains("beta turn"));
}

#[test]
fn dedicated_collection_mode_writes_conversations_outside_main_vault_root() {
    let vault_root = tempfile::TempDir::new().unwrap();
    let (_db_dir, _db_path, conn) = open_turn_db(vault_root.path());
    conn.execute(
        "UPDATE config SET value = 'dedicated-collection' WHERE key = 'memory.location'",
        [],
    )
    .unwrap();

    let result = append_turn(
        &conn,
        "session-1",
        TurnRole::Tool,
        "ran a tool",
        "2026-05-03T09:14:22Z",
        None,
        Some("alpha"),
    )
    .unwrap();

    let dedicated_root: String = conn
        .query_row(
            "SELECT root_path FROM collections WHERE name LIKE '%-memory'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let dedicated_path = PathBuf::from(dedicated_root);

    assert_eq!(
        result.conversation_path,
        "alpha/conversations/2026-05-03/session-1.md"
    );
    assert!(dedicated_path
        .join("alpha")
        .join("conversations")
        .join("2026-05-03")
        .join("session-1.md")
        .is_file());
    assert!(!vault_root
        .path()
        .join("alpha")
        .join("conversations")
        .join("2026-05-03")
        .join("session-1.md")
        .exists());
}

#[test]
fn append_turn_rejects_closed_day_file() {
    let vault_root = tempfile::TempDir::new().unwrap();
    let (_db_dir, _db_path, conn) = open_turn_db(vault_root.path());
    let path = vault_root.path().join("conversations").join("2026-05-03");
    fs::create_dir_all(&path).unwrap();
    fs::write(
        path.join("session-1.md"),
        concat!(
            "---\n",
            "type: conversation\n",
            "session_id: session-1\n",
            "date: 2026-05-03\n",
            "started_at: 2026-05-03T09:14:22Z\n",
            "status: closed\n",
            "last_extracted_at: null\n",
            "last_extracted_turn: 0\n",
            "---\n\n",
            "## Turn 1 · user · 2026-05-03T09:14:22Z\n\n",
            "done\n"
        ),
    )
    .unwrap();

    let error = append_turn(
        &conn,
        "session-1",
        TurnRole::Assistant,
        "should fail",
        "2026-05-03T09:15:00Z",
        None,
        None,
    )
    .unwrap_err();

    assert!(error.to_string().contains("closed"));
    assert_eq!(
        parse(&path.join("session-1.md"))
            .unwrap()
            .frontmatter
            .status,
        ConversationStatus::Closed
    );
}

#[test]
fn concurrent_appends_to_different_sessions_write_separate_files() {
    let vault_root = tempfile::TempDir::new().unwrap();
    let db_dir = tempfile::TempDir::new().unwrap();
    let db_path = db_dir.path().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    conn.execute(
        "UPDATE collections
         SET root_path = ?1,
             state = 'active'
         WHERE id = 1",
        [vault_root.path().display().to_string()],
    )
    .unwrap();
    drop(conn);

    let first_db = db_path.clone();
    let second_db = db_path.clone();
    let first = thread::spawn(move || {
        let conn = db::open(first_db.to_str().unwrap()).unwrap();
        append_turn(
            &conn,
            "session-a",
            TurnRole::User,
            "alpha",
            "2026-05-03T09:14:22Z",
            None,
            None,
        )
        .unwrap()
    });
    let second = thread::spawn(move || {
        let conn = db::open(second_db.to_str().unwrap()).unwrap();
        append_turn(
            &conn,
            "session-b",
            TurnRole::Assistant,
            "beta",
            "2026-05-03T09:14:23Z",
            None,
            None,
        )
        .unwrap()
    });

    let first = first.join().unwrap();
    let second = second.join().unwrap();

    assert_eq!(first.turn_id, "session-a:1");
    assert_eq!(second.turn_id, "session-b:1");
    assert!(vault_root
        .path()
        .join("conversations")
        .join("2026-05-03")
        .join("session-a.md")
        .is_file());
    assert!(vault_root
        .path()
        .join("conversations")
        .join("2026-05-03")
        .join("session-b.md")
        .is_file());
}

#[test]
fn append_turn_rejects_closed_session_on_following_day() {
    let vault_root = tempfile::TempDir::new().unwrap();
    let (_db_dir, _db_path, conn) = open_turn_db(vault_root.path());
    let path = vault_root.path().join("conversations").join("2026-05-03");
    fs::create_dir_all(&path).unwrap();
    fs::write(
        path.join("session-1.md"),
        concat!(
            "---\n",
            "type: conversation\n",
            "session_id: session-1\n",
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

    let error = append_turn(
        &conn,
        "session-1",
        TurnRole::Assistant,
        "should still fail",
        "2026-05-04T00:01:00Z",
        None,
        None,
    )
    .unwrap_err();

    assert!(error.to_string().contains("closed"));
    assert!(!vault_root
        .path()
        .join("conversations")
        .join("2026-05-04")
        .join("session-1.md")
        .exists());
}

#[test]
fn append_turn_serializes_same_session_writers_across_processes() {
    let vault_root = tempfile::TempDir::new().unwrap();
    let (db_dir, db_path, conn) = open_turn_db(vault_root.path());
    drop(conn);

    let current_exe = std::env::current_exe().unwrap();
    let signal_path = db_dir.path().join("append-turn.locked");
    let mut first = Command::new(&current_exe)
        .args(["--exact", "append_turn_cross_process_helper", "--nocapture"])
        .env(APPEND_TURN_HELPER_ENV, "1")
        .env(APPEND_TURN_DB_PATH_ENV, db_path.as_os_str())
        .env(APPEND_TURN_SESSION_ID_ENV, "shared-session")
        .env(APPEND_TURN_ROLE_ENV, "user")
        .env(APPEND_TURN_CONTENT_ENV, "first child")
        .env(APPEND_TURN_TIMESTAMP_ENV, "2026-05-03T09:14:22Z")
        .env("QUAID_TEST_APPEND_TURN_HOLD_MS", "600")
        .env(
            "QUAID_TEST_APPEND_TURN_LOCK_SIGNAL",
            signal_path.as_os_str(),
        )
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    while !signal_path.exists() {
        assert!(
            Instant::now() < deadline,
            "first child never acquired the session lock"
        );
        thread::sleep(Duration::from_millis(20));
    }

    let mut second = Command::new(&current_exe)
        .args(["--exact", "append_turn_cross_process_helper", "--nocapture"])
        .env(APPEND_TURN_HELPER_ENV, "1")
        .env(APPEND_TURN_DB_PATH_ENV, db_path.as_os_str())
        .env(APPEND_TURN_SESSION_ID_ENV, "shared-session")
        .env(APPEND_TURN_ROLE_ENV, "assistant")
        .env(APPEND_TURN_CONTENT_ENV, "second child")
        .env(APPEND_TURN_TIMESTAMP_ENV, "2026-05-03T09:14:23Z")
        .spawn()
        .unwrap();

    thread::sleep(Duration::from_millis(200));
    assert!(second.try_wait().unwrap().is_none());

    assert!(first.wait().unwrap().success());
    assert!(second.wait().unwrap().success());

    let parsed = parse(
        &vault_root
            .path()
            .join("conversations")
            .join("2026-05-03")
            .join("shared-session.md"),
    )
    .unwrap();

    assert_eq!(parsed.turns.len(), 2);
    assert_eq!(parsed.turns[0].ordinal, 1);
    assert_eq!(parsed.turns[0].content, "first child");
    assert_eq!(parsed.turns[1].ordinal, 2);
    assert_eq!(parsed.turns[1].content, "second child");
}
