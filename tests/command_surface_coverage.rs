#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use quaid::{
    commands::{link, put},
    core::{db, gaps},
};
use rusqlite::Connection;
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn init_db(dir: &tempfile::TempDir) -> PathBuf {
    let db_path = dir.path().join("memory.db");
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    let output = command
        .arg("init")
        .arg(&db_path)
        .output()
        .expect("run quaid init");
    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    db_path
}

fn run_quaid(db_path: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    command
        .arg("--db")
        .arg(db_path)
        .args(args)
        .output()
        .expect("run quaid")
}

fn run_quaid_in_dir(db_path: &Path, dir: &Path, args: &[&str], home_dir: &Path) -> Output {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    command
        .current_dir(dir)
        .env("HOME", home_dir)
        .env("USERPROFILE", home_dir)
        .arg("--db")
        .arg(db_path)
        .args(args)
        .output()
        .expect("run quaid in directory")
}

/// Provision a real vault root on the default collection (id=1).
///
/// Provision a real test-local vault root on the default collection (id=1).
///
/// Production init now provisions `~/.quaid/vault`, but these tests keep file
/// writes inside the temp directory so subprocess assertions remain isolated.
fn provision_vault(dir: &tempfile::TempDir, conn: &Connection) {
    let vault_root = dir.path().join("vault");
    fs::create_dir_all(&vault_root).unwrap();
    conn.execute(
        "UPDATE collections
         SET root_path = ?1,
             writable = 1,
             is_write_target = 1,
             state = 'active',
             needs_full_sync = 0
         WHERE id = 1",
        [vault_root.display().to_string()],
    )
    .unwrap();
}

fn run_quaid_with_input(db_path: &Path, args: &[&str], input: &str) -> Output {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    let mut child = command
        .arg("--db")
        .arg(db_path)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn quaid");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait for quaid")
}

#[test]
fn version_command_runs_without_opening_a_database() {
    let mut command = Command::new(common::quaid_bin());
    common_subprocess::configure_test_command(&mut command);
    let output = command.arg("version").output().expect("run quaid version");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("quaid"));
}

#[test]
fn config_commands_surface_missing_existing_and_list_values() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);

    let missing = run_quaid(&db_path, &["config", "get", "theme"]);
    assert!(missing.status.success());
    assert_eq!(String::from_utf8_lossy(&missing.stdout).trim(), "Not set");

    let set = run_quaid(&db_path, &["config", "set", "theme", "dark"]);
    assert!(set.status.success());
    assert!(
        String::from_utf8_lossy(&set.stdout).contains("Set theme = dark"),
        "set stdout: {}",
        String::from_utf8_lossy(&set.stdout)
    );

    let existing = run_quaid(&db_path, &["config", "get", "theme"]);
    assert!(existing.status.success());
    assert_eq!(String::from_utf8_lossy(&existing.stdout).trim(), "dark");

    let list = run_quaid(&db_path, &["config", "list"]);
    assert!(list.status.success());
    assert!(String::from_utf8_lossy(&list.stdout).contains("theme=dark"));
}

#[test]
fn export_command_writes_markdown_files_for_existing_pages() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    provision_vault(&dir, &conn);
    put::put_from_string(
        &conn,
        "notes/exported",
        "---\ntitle: Exported Page\n---\nBody from export test\n",
        None,
    )
    .unwrap();
    drop(conn);

    let export_dir = dir.path().join("out");
    let output = run_quaid(
        &db_path,
        &["export", export_dir.to_str().expect("export path")],
    );
    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Exported 1 page(s)"),
        "export stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );

    let exported = export_dir.join("notes").join("exported.md");
    let contents = fs::read_to_string(exported).unwrap();
    assert!(contents.contains("title: Exported Page"));
    assert!(contents.contains("Body from export test"));
}

#[test]
fn collection_add_rejects_double_colon_name_during_clap_parse() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let root = dir.path().join("vault");
    fs::create_dir_all(&root).unwrap();

    let output = run_quaid(
        &db_path,
        &["collection", "add", "bad::name", root.to_str().unwrap()],
    );

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("collection name cannot contain '::'"));
}

#[test]
fn skills_commands_report_shadowing_and_frontmatter_issues() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let home_dir = dir.path().join("home");
    let project_dir = dir.path().join("workspace");
    fs::create_dir_all(&home_dir).unwrap();
    fs::create_dir_all(&project_dir).unwrap();

    let ingest_dir = project_dir.join("skills").join("ingest");
    fs::create_dir_all(&ingest_dir).unwrap();
    fs::write(ingest_dir.join("SKILL.md"), "---\nname: ingest\n---\n").unwrap();

    let local_skill_dir = project_dir.join("skills").join("local-extra");
    fs::create_dir_all(&local_skill_dir).unwrap();
    fs::write(
        local_skill_dir.join("SKILL.md"),
        "---\nname: local-extra\ndescription: local extra\n---\n",
    )
    .unwrap();

    let list = run_quaid_in_dir(
        &db_path,
        &project_dir,
        &["skills", "list", "--json"],
        &home_dir,
    );
    assert!(list.status.success());
    let listed: Value = serde_json::from_slice(&list.stdout).unwrap();
    let listed = listed.as_array().unwrap();
    assert!(listed.iter().any(|skill| skill["name"] == "local-extra"));
    assert!(listed
        .iter()
        .any(|skill| { skill["name"] == "ingest" && skill["shadowed"].as_bool() == Some(true) }));

    let list_text = run_quaid_in_dir(&db_path, &project_dir, &["skills", "list"], &home_dir);
    assert!(list_text.status.success());
    let list_stdout = String::from_utf8_lossy(&list_text.stdout);
    assert!(list_stdout.contains("local-extra"));
    assert!(list_stdout.contains("ingest"));
    assert!(list_stdout.contains("skill(s) resolved."));

    let doctor = run_quaid_in_dir(
        &db_path,
        &project_dir,
        &["skills", "doctor", "--json"],
        &home_dir,
    );
    assert!(doctor.status.success());
    let diagnosed: Value = serde_json::from_slice(&doctor.stdout).unwrap();
    let diagnosed = diagnosed.as_array().unwrap();
    let ingest = diagnosed
        .iter()
        .find(|skill| skill["name"] == "ingest")
        .expect("doctor result for ingest");
    assert_eq!(ingest["valid_frontmatter"], true);
    assert_eq!(ingest["has_name"], true);
    assert_eq!(ingest["has_description"], false);
    assert!(ingest["issues"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue == "frontmatter missing 'description' field"));

    let doctor_text = run_quaid_in_dir(&db_path, &project_dir, &["skills", "doctor"], &home_dir);
    assert!(doctor_text.status.success());
    let doctor_stdout = String::from_utf8_lossy(&doctor_text.stdout);
    assert!(doctor_stdout.contains("frontmatter missing 'description' field"));
    assert!(doctor_stdout.contains("skill(s) with issues"));
}

#[test]
fn stats_compact_validate_and_gaps_commands_cover_reporting_surface() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);

    let stats = run_quaid(&db_path, &["stats", "--json"]);
    assert!(stats.status.success());
    let stats_json: Value = serde_json::from_slice(&stats.stdout).unwrap();
    assert_eq!(stats_json["total_pages"], 0);

    let compact = run_quaid(&db_path, &["compact"]);
    assert!(compact.status.success());
    assert!(String::from_utf8_lossy(&compact.stdout).contains("Compacted database"));

    let validate = run_quaid(&db_path, &["validate", "--links"]);
    assert!(validate.status.success());
    assert!(String::from_utf8_lossy(&validate.stdout).contains("All checks passed."));

    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    gaps::log_gap(
        None,
        "coverage-gap",
        "surface coverage gap",
        Some(0.25),
        &conn,
    )
    .unwrap();
    drop(conn);

    let gap_output = run_quaid(&db_path, &["gaps", "--json"]);
    assert!(gap_output.status.success());
    let gaps_json: Value = serde_json::from_slice(&gap_output.stdout).unwrap();
    assert_eq!(gaps_json.as_array().map(Vec::len), Some(1));
}

#[test]
fn daemon_status_status_and_serve_http_fail_closed_run_through_main_dispatch() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let home_dir = dir.path().join("home");
    fs::create_dir_all(&home_dir).unwrap();

    let status = run_quaid_in_dir(&db_path, dir.path(), &["status", "--json"], &home_dir);
    assert_eq!(status.status.code(), Some(2));
    let status_json: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status_json["daemon"]["installed"], false);
    assert_eq!(status_json["daemon"]["running"], false);

    let daemon_status = run_quaid_in_dir(
        &db_path,
        dir.path(),
        &["daemon", "status", "--json"],
        &home_dir,
    );
    assert_eq!(daemon_status.status.code(), Some(2));
    let daemon_json: Value = serde_json::from_slice(&daemon_status.stdout).unwrap();
    assert_eq!(daemon_json["installed"], false);
    assert_eq!(daemon_json["running"], false);

    let serve_http = run_quaid(&db_path, &["serve", "--http"]);
    assert!(!serve_http.status.success());
    assert!(String::from_utf8_lossy(&serve_http.stderr).contains("requires --trust-loopback"));
}

#[test]
fn ingest_command_accepts_markdown_sources() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);

    let ingest_file = dir.path().join("ingest.md");
    fs::write(
        &ingest_file,
        "---\nslug: notes/ingested\ntitle: Ingested\n---\nIngest body\n",
    )
    .unwrap();
    let ingest = run_quaid(&db_path, &["ingest", ingest_file.to_str().unwrap()]);
    assert!(
        ingest.status.success(),
        "ingest stderr: {}",
        String::from_utf8_lossy(&ingest.stderr)
    );
    assert!(String::from_utf8_lossy(&ingest.stdout).contains("Ingested notes/ingested"));
}

#[test]
fn tags_timeline_add_and_link_close_commands_update_existing_records() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    provision_vault(&dir, &conn);
    put::put_from_string(
        &conn,
        "notes/alpha",
        "---\ntitle: Alpha\n---\nAlpha\n",
        None,
    )
    .unwrap();
    put::put_from_string(&conn, "notes/beta", "---\ntitle: Beta\n---\nBeta\n", None).unwrap();
    link::run_silent(
        &conn,
        "notes/alpha",
        "notes/beta",
        "related",
        Some("2026-04-28".to_owned()),
        None,
    )
    .unwrap();
    let link_id: i64 = conn
        .query_row("SELECT id FROM links LIMIT 1", [], |row| row.get(0))
        .unwrap();
    drop(conn);

    let tags = run_quaid(
        &db_path,
        &[
            "tags",
            "notes/alpha",
            "--add",
            "focus",
            "--add",
            "important",
        ],
    );
    assert!(
        tags.status.success(),
        "tags stderr: {}",
        String::from_utf8_lossy(&tags.stderr)
    );

    let timeline = run_quaid(
        &db_path,
        &[
            "timeline-add",
            "notes/alpha",
            "--date",
            "2026-04-28",
            "--summary",
            "Coverage landed",
            "--source",
            "scruffy",
            "--detail",
            "main arm covered",
        ],
    );
    assert!(
        timeline.status.success(),
        "timeline stderr: {}",
        String::from_utf8_lossy(&timeline.stderr)
    );

    let close = run_quaid(
        &db_path,
        &[
            "link-close",
            &link_id.to_string(),
            "--valid-until",
            "2026-04-29",
        ],
    );
    assert!(
        close.status.success(),
        "link-close stderr: {}",
        String::from_utf8_lossy(&close.stderr)
    );

    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    let tag_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tags WHERE tag IN ('focus', 'important')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tag_count, 2);
    let timeline_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM timeline_entries", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(timeline_count, 1);
    let valid_until: String = conn
        .query_row(
            "SELECT valid_until FROM links WHERE id = ?1",
            [link_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(valid_until, "2026-04-29");
}

#[test]
fn read_graph_and_namespace_commands_run_through_main_dispatch() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let conn = db::open(db_path.to_str().unwrap()).unwrap();
    provision_vault(&dir, &conn);
    put::put_from_string(
        &conn,
        "notes/alpha",
        "---\ntitle: Alpha\n---\nAlpha body about rivers\n",
        None,
    )
    .unwrap();
    put::put_from_string(
        &conn,
        "notes/beta",
        "---\ntitle: Beta\n---\nBeta body mentions Alpha\n",
        None,
    )
    .unwrap();
    link::run_silent(
        &conn,
        "notes/alpha",
        "notes/beta",
        "related",
        Some("2026-04-28".to_owned()),
        None,
    )
    .unwrap();
    drop(conn);

    let get = run_quaid(&db_path, &["get", "notes/alpha"]);
    assert!(
        get.status.success(),
        "get stderr: {}",
        String::from_utf8_lossy(&get.stderr)
    );
    assert!(String::from_utf8_lossy(&get.stdout).contains("Alpha body"));

    let list = run_quaid(&db_path, &["list", "--limit", "10"]);
    assert!(
        list.status.success(),
        "list stderr: {}",
        String::from_utf8_lossy(&list.stderr)
    );
    assert!(String::from_utf8_lossy(&list.stdout).contains("notes/alpha"));

    let search = run_quaid(&db_path, &["search", "rivers"]);
    assert!(
        search.status.success(),
        "search stderr: {}",
        String::from_utf8_lossy(&search.stderr)
    );
    assert!(String::from_utf8_lossy(&search.stdout).contains("notes/alpha"));

    let embed = run_quaid(&db_path, &["embed", "--all"]);
    assert!(
        embed.status.success(),
        "embed stderr: {}",
        String::from_utf8_lossy(&embed.stderr)
    );

    let query = run_quaid(&db_path, &["query", "alpha river"]);
    assert!(
        query.status.success(),
        "query stderr: {}",
        String::from_utf8_lossy(&query.stderr)
    );

    let links = run_quaid(&db_path, &["links", "notes/alpha"]);
    assert!(
        links.status.success(),
        "links stderr: {}",
        String::from_utf8_lossy(&links.stderr)
    );
    assert!(String::from_utf8_lossy(&links.stdout).contains("notes/beta"));

    let backlinks = run_quaid(&db_path, &["backlinks", "notes/beta"]);
    assert!(
        backlinks.status.success(),
        "backlinks stderr: {}",
        String::from_utf8_lossy(&backlinks.stderr)
    );
    assert!(String::from_utf8_lossy(&backlinks.stdout).contains("notes/alpha"));

    let graph = run_quaid(
        &db_path,
        &["graph", "notes/alpha", "--depth", "1", "--json"],
    );
    assert!(
        graph.status.success(),
        "graph stderr: {}",
        String::from_utf8_lossy(&graph.stderr)
    );
    let graph_json: Value = serde_json::from_slice(&graph.stdout).unwrap();
    assert!(graph_json["nodes"].as_array().is_some());

    let check = run_quaid(&db_path, &["check", "notes/alpha"]);
    assert!(
        check.status.success(),
        "check stderr: {}",
        String::from_utf8_lossy(&check.stderr)
    );

    let unlink = run_quaid(
        &db_path,
        &[
            "unlink",
            "notes/alpha",
            "notes/beta",
            "--relationship",
            "related",
        ],
    );
    assert!(
        unlink.status.success(),
        "unlink stderr: {}",
        String::from_utf8_lossy(&unlink.stderr)
    );

    let call = run_quaid(&db_path, &["call", "memory_list", "{\"limit\":5}"]);
    assert!(
        call.status.success(),
        "call stderr: {}",
        String::from_utf8_lossy(&call.stderr)
    );

    let namespace_create = run_quaid(&db_path, &["namespace", "create", "scratch"]);
    assert!(
        namespace_create.status.success(),
        "namespace create stderr: {}",
        String::from_utf8_lossy(&namespace_create.stderr)
    );

    let namespace_list = run_quaid(&db_path, &["namespace", "list", "--json"]);
    assert!(
        namespace_list.status.success(),
        "namespace list stderr: {}",
        String::from_utf8_lossy(&namespace_list.stderr)
    );
    let namespace_json: Value = serde_json::from_slice(&namespace_list.stdout).unwrap();
    assert_eq!(namespace_json.as_array().map(Vec::len), Some(1));

    let namespace_destroy = run_quaid(&db_path, &["namespace", "destroy", "scratch"]);
    assert!(
        namespace_destroy.status.success(),
        "namespace destroy stderr: {}",
        String::from_utf8_lossy(&namespace_destroy.stderr)
    );
}

#[test]
fn embed_and_pipe_commands_run_through_main_dispatch() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);

    let embed = run_quaid(&db_path, &["embed", "--all"]);
    assert!(
        embed.status.success(),
        "embed stderr: {}",
        String::from_utf8_lossy(&embed.stderr)
    );
    assert!(String::from_utf8_lossy(&embed.stdout).contains("Embedded 0 chunks across 0 page(s)."));

    let pipe = run_quaid_with_input(
        &db_path,
        &["pipe"],
        "{\"tool\":\"memory_stats\",\"input\":{}}\n",
    );
    assert!(
        pipe.status.success(),
        "pipe stderr: {}",
        String::from_utf8_lossy(&pipe.stderr)
    );
    let pipe_json: Value = serde_json::from_slice(&pipe.stdout).unwrap();
    assert!(pipe_json.get("page_count").is_some());
}

#[cfg(windows)]
#[test]
fn serve_command_fails_closed_on_windows() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);

    let output = run_quaid(&db_path, &["serve"]);
    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("expect initialize request") || stderr.contains("unsupported"),
        "unexpected stderr: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn collection_migrate_uuids_command_runs_through_main_dispatch() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let root = dir.path().join("vault");
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("note.md"),
        "---\ntitle: Note\ntype: note\n---\ncommand coverage\n",
    )
    .unwrap();

    let add = run_quaid(
        &db_path,
        &["collection", "add", "work", root.to_str().unwrap()],
    );
    assert!(
        add.status.success(),
        "collection add stderr: {}",
        String::from_utf8_lossy(&add.stderr)
    );

    let migrate = run_quaid(
        &db_path,
        &["collection", "migrate-uuids", "work", "--dry-run", "--json"],
    );
    assert!(
        migrate.status.success(),
        "migrate stderr: {}",
        String::from_utf8_lossy(&migrate.stderr)
    );
    let migrate_json: Value = serde_json::from_slice(&migrate.stdout).unwrap();
    assert_eq!(migrate_json["migrated"], 1);
    assert_eq!(migrate_json["skipped_readonly"], 0);
    assert_eq!(migrate_json["already_had_uuid"], 0);
}

#[cfg(windows)]
#[test]
fn collection_restore_sync_remap_and_audit_fail_closed_on_windows() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = init_db(&dir);
    let remap_root = dir.path().join("remapped");
    let restore_root = dir.path().join("restored");
    fs::create_dir_all(&remap_root).unwrap();

    let sync = run_quaid(
        &db_path,
        &[
            "collection",
            "sync",
            "default",
            "--remap-root",
            remap_root.to_str().unwrap(),
        ],
    );
    assert!(
        !sync.status.success(),
        "collection sync must fail closed: {sync:?}"
    );

    let restore = run_quaid(
        &db_path,
        &[
            "collection",
            "restore",
            "default",
            restore_root.to_str().unwrap(),
        ],
    );
    assert!(
        !restore.status.success(),
        "collection restore must fail closed: {restore:?}"
    );

    let audit = run_quaid(&db_path, &["collection", "audit", "default"]);
    assert!(
        !audit.status.success(),
        "collection audit must fail closed: {audit:?}"
    );

    for output in [&sync, &restore, &audit] {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("UnsupportedPlatformError")
                || stderr.contains("Vault sync commands require Unix"),
            "Windows collection surface must report the Unix gate clearly: {stderr}"
        );
    }
}
