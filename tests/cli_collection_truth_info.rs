#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Integration tests for `quaid collection info` truth-merge behavior.

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;
#[path = "common/truth_fixtures.rs"]
mod truth_fixtures;

use rusqlite::params;
use truth_fixtures::*;

#[test]
fn collection_info_json_reports_restore_integrity_blockers() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "collection-info-integrity.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             pending_root_path = ?2,
             integrity_failed_at = '2026-04-23T00:00:00Z',
             pending_manifest_incomplete_at = '2026-04-23T00:05:00Z'
         WHERE id = ?1",
        params![
            collection_id,
            dir.path().join("restored").display().to_string()
        ],
    )
    .expect("seed integrity blockers");
    drop(conn);

    let output = run_quaid(&db_path, &["--json", "collection", "info", "work"]);

    assert!(
        output.status.success(),
        "collection info should succeed: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert_eq!(parsed["state"].as_str(), Some("restoring"));
    assert_eq!(
        parsed["blocked_state"].as_str(),
        Some("restore_integrity_blocked")
    );
    assert_eq!(
        parsed["integrity_blocked"].as_str(),
        Some("manifest_tampering")
    );
    assert_eq!(
        parsed["integrity_failed_at"].as_str(),
        Some("2026-04-23T00:00:00Z")
    );
    assert_eq!(
        parsed["pending_manifest_incomplete_at"].as_str(),
        Some("2026-04-23T00:05:00Z")
    );
    assert!(parsed["pending_root_path"].as_str().is_some());
    assert_eq!(
        parsed["suggested_command"].as_str(),
        Some("quaid collection restore-reset work --confirm")
    );
}

#[test]
fn collection_info_json_points_retryable_manifest_gap_to_finalize_pending() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "collection-info-manifest-gap.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             pending_root_path = ?2,
             pending_manifest_incomplete_at = '2026-04-23T00:05:00Z'
         WHERE id = ?1",
        params![
            collection_id,
            dir.path().join("restored").display().to_string()
        ],
    )
    .expect("seed retryable manifest gap");
    drop(conn);

    let output = run_quaid(&db_path, &["--json", "collection", "info", "work"]);

    assert!(
        output.status.success(),
        "collection info should succeed: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert_eq!(parsed["blocked_state"].as_str(), Some("pending_finalize"));
    assert_eq!(
        parsed["integrity_blocked"].as_str(),
        Some("manifest_incomplete_pending")
    );
    assert_eq!(
        parsed["suggested_command"].as_str(),
        Some("quaid collection sync work --finalize-pending")
    );
}

#[test]
fn collection_info_json_reports_reconcile_halt_cause() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "collection-info-halt.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    conn.execute(
        "UPDATE collections
         SET reconcile_halted_at = '2026-04-23T00:10:00Z',
             reconcile_halt_reason = 'duplicate_uuid'
         WHERE id = ?1",
        [collection_id],
    )
    .expect("seed reconcile halt");
    drop(conn);

    let output = run_quaid(&db_path, &["--json", "collection", "info", "work"]);

    assert!(
        output.status.success(),
        "collection info should succeed: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert_eq!(parsed["blocked_state"].as_str(), Some("reconcile_halted"));
    assert_eq!(parsed["integrity_blocked"].as_str(), Some("duplicate_uuid"));
    assert_eq!(
        parsed["reconcile_halted_at"].as_str(),
        Some("2026-04-23T00:10:00Z")
    );
    assert_eq!(
        parsed["reconcile_halt_reason"].as_str(),
        Some("duplicate_uuid")
    );
    assert_eq!(
        parsed["suggested_command"].as_str(),
        Some("quaid collection reconcile-reset work --confirm")
    );
}

#[test]
fn collection_info_json_reports_read_only_truthfully() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "collection-info-read-only.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    conn.execute(
        "UPDATE collections
         SET writable = 0,
             last_sync_at = '2026-04-23T00:15:00Z'
         WHERE id = ?1",
        [collection_id],
    )
    .expect("seed read-only collection");
    drop(conn);

    let output = run_quaid(&db_path, &["--json", "collection", "info", "work"]);

    assert!(
        output.status.success(),
        "collection info should succeed: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert_eq!(parsed["name"].as_str(), Some("work"));
    assert_eq!(parsed["state"].as_str(), Some("active"));
    assert_eq!(parsed["writable"].as_bool(), Some(false));
    assert_eq!(
        parsed["last_sync_at"].as_str(),
        Some("2026-04-23T00:15:00Z")
    );
}

#[test]
fn collection_info_json_reports_quarantine_backlog_count() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "collection-info-quarantine.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    insert_page(&conn, collection_id, "notes/active");
    insert_page(&conn, collection_id, "notes/quarantined");
    quarantine_page(
        &conn,
        page_id(&conn, collection_id, "notes/quarantined"),
        "2026-04-25T00:00:00Z",
    );
    drop(conn);

    let output = run_quaid(&db_path, &["--json", "collection", "info", "work"]);

    assert!(
        output.status.success(),
        "collection info should succeed: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert_eq!(
        parsed["quarantined_pages_awaiting_action"].as_i64(),
        Some(1)
    );
}

#[test]
fn collection_info_json_reports_null_watcher_health_without_live_runtime_registry() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "collection-info-watcher-health-null.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let _collection_id = insert_collection(&conn, "work", &root);
    drop(conn);

    let output = run_quaid(&db_path, &["--json", "collection", "info", "work"]);

    assert!(
        output.status.success(),
        "collection info should succeed: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert!(parsed["watcher_mode"].is_null());
    assert!(parsed["watcher_last_event_at"].is_null());
    assert!(parsed["watcher_channel_depth"].is_null());
}

#[test]
fn collection_info_json_reports_release_metadata_queue_depth_and_active_reconcile_status() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "collection-info-release-metadata.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    std::fs::create_dir_all(&root).expect("create root");
    let collection_id = insert_collection(&conn, "work", &root);
    insert_page(&conn, collection_id, "notes/a");
    insert_page(&conn, collection_id, "notes/b");
    let work_page_id = page_id(&conn, collection_id, "notes/a");
    let failed_page_id = page_id(&conn, collection_id, "notes/b");
    conn.execute(
        "INSERT INTO embedding_jobs (page_id) VALUES (?1)",
        [work_page_id],
    )
    .expect("insert embedding job");
    conn.execute(
        "INSERT INTO embedding_jobs (page_id, job_state, attempt_count, last_error)
         VALUES (?1, 'failed', 5, 'hash shim exploded')",
        [failed_page_id],
    )
    .expect("insert failed embedding job");
    conn.execute(
        "UPDATE collections
         SET needs_full_sync = 1,
             ignore_parse_errors = 'line 3 raw=\"[broken\" error=Invalid glob pattern',
             reload_generation = 4,
             watcher_released_session_id = 'serve-1',
             watcher_released_generation = 3
         WHERE id = ?1",
        [collection_id],
    )
    .expect("seed release metadata");
    drop(conn);

    let output = run_quaid(&db_path, &["--json", "collection", "info", "work"]);

    assert!(
        output.status.success(),
        "collection info should succeed: {output:?}"
    );
    let parsed = parse_stdout_json(&output);
    assert_eq!(
        parsed["blocked_state"].as_str(),
        Some("active_reconcile_needed")
    );
    assert_eq!(
        parsed["suggested_command"].as_str(),
        Some("quaid collection sync work")
    );
    assert_eq!(
        parsed["status_message"].as_str(),
        Some("collection is active but needs a real reconcile before writes are considered fully healthy")
    );
    assert_eq!(parsed["queue_depth"].as_i64(), Some(1));
    assert_eq!(parsed["failing_jobs"].as_i64(), Some(1));
    assert_eq!(parsed["reload_generation"].as_i64(), Some(4));
    assert_eq!(
        parsed["watcher_released_session_id"].as_str(),
        Some("serve-1")
    );
    assert_eq!(parsed["watcher_released_generation"].as_i64(), Some(3));
    assert_eq!(
        parsed["ignore_parse_errors"].as_str(),
        Some("line 3 raw=\"[broken\" error=Invalid glob pattern")
    );
    assert!(parsed["watcher_mode"].is_null());
}

#[test]
fn collection_info_plain_text_reports_retryable_manifest_gap_truthfully() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let db_path = test_db_path(&dir, "collection-info-plain.db");
    let conn = open_test_db(&db_path);
    let root = dir.path().join("vault");
    let pending_root = dir.path().join("restored");
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::create_dir_all(&pending_root).expect("create pending root");
    let collection_id = insert_collection(&conn, "work", &root);
    conn.execute(
        "UPDATE collections
         SET state = 'restoring',
             pending_root_path = ?2,
             pending_manifest_incomplete_at = '2026-04-23T00:05:00Z',
             reload_generation = 9
         WHERE id = ?1",
        params![collection_id, pending_root.display().to_string()],
    )
    .expect("seed retryable manifest gap");
    drop(conn);

    let output = run_quaid(&db_path, &["collection", "info", "work"]);

    assert!(
        output.status.success(),
        "collection info should succeed: {output:?}"
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("collection=work state=restoring writable=writable"));
    assert!(text.contains("pending_root_path="));
    assert!(
        text.contains("watcher_mode=null watcher_last_event_at=null watcher_channel_depth=null")
    );
    assert!(text.contains("blocked_state=pending_finalize"));
    assert!(text.contains("integrity_blocked=manifest_incomplete_pending"));
    assert!(text.contains("suggested_command=quaid collection sync work --finalize-pending"));
    assert!(text.contains("status_message=\"restore manifest is still incomplete; collection remains blocked until the files reappear and quaid collection sync work --finalize-pending succeeds\""));
}
