#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Source-text invariants for `src/commands/put.rs` (and the
//! `vault_sync` helper it depends on).
//!
//! These tests do not exercise behaviour at runtime — they read the
//! production source files via `CARGO_MANIFEST_DIR` and assert the
//! seam structure that the surrounding behaviour tests rely on:
//!
//! * `put_from_cli_string` proxies through the live-serve socket
//!   before considering an offline lease, and the live-owner branch
//!   returns immediately after the proxy.
//! * `proxy_put_via_live_serve` performs kernel peer-credential
//!   verification before issuing the IPC `WhoAmI` request, and
//!   guards against socket-mode/uid/pid spoof attempts.
//! * The Unix writer `persist_with_vault_write` keeps its
//!   fd-relative ordering of the rename-before-commit seam, and
//!   never widens parent creation back to path-based
//!   `fs::create_dir_all`.
//! * Duplicate `insert_write_dedup` failures are fail-closed and do
//!   not erase the pre-existing dedup entry or unrelated self-write
//!   tracking state.

use std::fs as stdfs;
use std::path::Path;

#[test]
fn cli_put_source_routes_live_owner_through_ipc_without_direct_fallback() {
    let source = stdfs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("commands")
            .join("put.rs"),
    )
    .unwrap();
    let cli_start = source
        .find("fn put_from_cli_string(")
        .expect("cli put source present");
    let cli_end = source[cli_start..]
        .find("/// Apply page content supplied by the caller.")
        .map(|offset| cli_start + offset)
        .expect("cli helper boundary");
    let cli_source = &source[cli_start..cli_end];
    let proxy_idx = cli_source
        .find("vault_sync::live_serve_endpoint_for_root_path")
        .expect("live serve owner lookup");
    let direct_idx = cli_source
        .find("start_short_lived_owner_lease_for_root_path")
        .expect("offline lease path");
    assert!(
        proxy_idx < direct_idx,
        "live owner check must precede offline lease"
    );
    assert!(
        cli_source.contains(
            "proxy_put_via_live_serve(&endpoint, &canonical_slug, content, expected_version)?;"
        ),
        "live owner branch must proxy through IPC"
    );
    let return_idx = cli_source[proxy_idx..]
        .find("return Ok(());")
        .map(|offset| proxy_idx + offset)
        .expect("live owner branch returns immediately after proxy");
    assert!(
        return_idx < direct_idx,
        "live owner branch must return after IPC proxy instead of falling back to direct write"
    );
}

#[test]
fn proxy_put_source_verifies_kernel_peer_before_whoami() {
    let source = stdfs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("commands")
            .join("put.rs"),
    )
    .unwrap();
    let proxy_start = source
        .find("fn proxy_put_via_live_serve(")
        .expect("proxy helper present");
    let proxy_end = source[proxy_start..]
        .find("fn send_ipc_request(")
        .map(|offset| proxy_start + offset)
        .expect("proxy helper boundary");
    let proxy_source = &source[proxy_start..proxy_end];
    let peer_idx = proxy_source
        .find("vault_sync::peer_credentials_for_stream(&stream)")
        .expect("peer credential call present");
    let whoami_idx = proxy_source
        .find("send_ipc_request(&mut stream, &vault_sync::IpcRequest::WhoAmI)")
        .expect("whoami call present");
    assert!(
        peer_idx < whoami_idx,
        "kernel peer auth must happen before whoami"
    );
    assert!(proxy_source.contains("socket mode {:o} is not 600"));
    assert!(proxy_source.contains("socket uid {} does not match current uid {}"));
    assert!(proxy_source.contains("peer pid {} does not match owner pid {}"));
}

#[test]
fn rename_before_commit_source_keeps_fd_relative_ordering() {
    let source = stdfs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("commands")
            .join("put.rs"),
    )
    .unwrap();
    let writer_start = source
        .match_indices("fn persist_with_vault_write(")
        .nth(1)
        .map(|(index, _)| index)
        .expect("unix writer function in production source");
    let production = &source[writer_start
        ..source
            .find("fn slug_to_relative_path(")
            .expect("helper boundary after writer function")];
    let required_sequence = [
        "vault_sync::check_update_expected_version(",
        "fs_safety::open_root_fd(Path::new(&collection.root_path))",
        "fs_safety::walk_to_parent_create_dirs(&root_fd, &relative_path_buf)",
        "vault_sync::check_fs_precondition_with_parent_fd(",
        "let tx = match db.unchecked_transaction()",
        "let staged = match stage_page_record(&tx, prepared, expected_version)",
        "create_recovery_sentinel(",
        "create_tempfile(&parent_fd, &temp_name, raw_bytes)",
        "fs_safety::stat_at_nofollow(&parent_fd, target_name)",
        "vault_sync::insert_write_dedup(&dedup_key)",
        "fs_safety::renameat_parent_fd(&parent_fd, &temp_name, target_name)",
        "sync_fd(&parent_fd)",
        "file_state::stat_file_fd(&parent_fd, target_name)",
        "let outcome = match commit_staged_page_record(",
        "let _ = vault_sync::remove_write_dedup(&dedup_key);",
    ];
    let mut last_index = 0;
    for snippet in required_sequence {
        let index = production
            .find(snippet)
            .unwrap_or_else(|| panic!("missing production seam snippet: {snippet}"));
        assert!(
            index >= last_index,
            "rename-before-commit seam reordered: `{snippet}` moved before an earlier step"
        );
        last_index = index;
    }
    let sentinel_cleanup = production
        .rfind("let _ = remove_recovery_sentinel(&recovery_dir, &sentinel_name);")
        .expect("final sentinel cleanup after commit");
    assert!(
        sentinel_cleanup >= last_index,
        "rename-before-commit seam reordered: final sentinel cleanup moved before the SQLite commit"
    );
    assert!(
        !production.contains("fs::create_dir_all(parent)"),
        "rename-before-commit writer must not widen parent creation back to path-based create_dir_all"
    );
}

#[test]
fn duplicate_dedup_source_is_fail_closed_without_clearing_preexisting_entry() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let vault_source = stdfs::read_to_string(root.join("core").join("vault_sync.rs")).unwrap();
    let insert_start = vault_source
        .find("pub fn insert_write_dedup(key: &str) -> Result<(), VaultSyncError> {")
        .expect("insert_write_dedup production function");
    let insert_end = vault_source[insert_start..]
        .find("pub fn remove_write_dedup")
        .map(|offset| insert_start + offset)
        .expect("remove_write_dedup after insert_write_dedup");
    let insert_fn = &vault_source[insert_start..insert_end];
    assert!(
        insert_fn.contains("Err(VaultSyncError::DuplicateWriteDedup"),
        "duplicate write-dedup entries must fail closed with an explicit typed error"
    );
    assert!(
        insert_fn.contains("if inserted {"),
        "insert_write_dedup must branch on the HashSet::insert result instead of silently discarding duplicates"
    );

    let put_source = stdfs::read_to_string(root.join("commands").join("put.rs")).unwrap();
    let dedup_fail_start = put_source
        .find("if let Err(error) = vault_sync::insert_write_dedup(&dedup_key) {")
        .expect("writer dedup failure block");
    let dedup_fail_end = put_source[dedup_fail_start..]
        .find("if let Err(error) = vault_sync::remember_self_write_path(&target_path, &prepared.sha256)")
        .map(|offset| dedup_fail_start + offset)
        .expect("self-write remember block after dedup failure block");
    let dedup_fail_block = &put_source[dedup_fail_start..dedup_fail_end];
    assert!(
        dedup_fail_block.contains("cleanup_pre_rename_without_dedup_clear("),
        "duplicate dedup insert failures must clean tempfile/sentinel without erasing the preexisting registry entry"
    );
    assert!(
        !dedup_fail_block.contains("let _ = vault_sync::forget_self_write_path(&target_path);"),
        "dedup insert failure happens before self-write tracking and must not clear unrelated path state"
    );
    assert!(
        !dedup_fail_block.contains("cleanup_pre_rename("),
        "dedup insert failure must not run the generic cleanup path that removes the preexisting dedup entry"
    );
}
