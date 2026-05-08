#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Source-text static-analysis tests that pin reconciler invariants.

#[path = "common/reconciler_fixtures.rs"]
mod common_reconciler_fixtures;

use common_reconciler_fixtures::*;

#[test]
fn remap_safety_pipeline_wrapper_source_skips_mount_verifier_only() {
    let source = production_reconciler_source();
    let start = source
        .find("pub(crate) fn run_restore_remap_safety_pipeline_without_mount_check(")
        .unwrap();
    let end = source[start..]
        .find("pub fn fresh_attach_reconcile_and_activate(")
        .map(|offset| start + offset)
        .unwrap();
    let snippet = &source[start..end];

    assert!(
        snippet.contains("run_restore_remap_safety_pipeline_inner(conn, request, |_| Ok(()), || Ok(()))"),
        "the no-mount wrapper must reuse the shared safety pipeline and only skip the mount verifier"
    );
}

#[test]
fn remap_safety_pipeline_source_keeps_phase_order_before_dirty_recheck() {
    let source = production_reconciler_source();
    let start = source
        .find("fn run_restore_remap_safety_pipeline_inner")
        .expect("shared restore/remap safety pipeline must remain present");
    let end = source[start..]
        .find("pub fn run_restore_remap_safety_pipeline(")
        .map(|offset| start + offset)
        .expect("shared restore/remap safety wrapper must remain present");
    let snippet = &source[start..end];
    let phase1_idx = snippet
        .find("let mut total_drift =")
        .expect("phase 1 drift capture assignment must remain present");
    let phase2_idx = snippet
        .find("run_phase2_stability_check(")
        .expect("phase 2 stability check must remain present");
    let phase3_idx = snippet
        .find("run_phase3_pre_destruction_fence(")
        .expect("phase 3 fence must remain present");
    let dirty_idx = snippet
        .find("fresh_collection_dirty_status(")
        .expect("fresh-connection dirty recheck must remain present");
    assert!(
        phase1_idx < phase2_idx && phase2_idx < phase3_idx && phase3_idx < dirty_idx,
        "restore/remap safety must capture drift, prove stability, fence, then do the fresh dirty recheck"
    );
}

#[test]
fn scheduled_full_hash_audit_source_uses_nofollow_fd_relative_reads() {
    let source = production_reconciler_source();
    let start = source
        .find("pub fn scheduled_full_hash_audit_authorized(")
        .expect("scheduled audit entrypoint must remain present");
    let end = source[start..]
        .find("fn authorize_full_hash_reconcile(")
        .map(|offset| start + offset)
        .expect("audit helper block must remain before authorization logic");
    let snippet = &source[start..end];

    assert!(
        snippet.contains("let root_fd = fs_safety::open_root_fd(root_path)?;")
            && snippet.contains("stat_and_hash_audit_path(&root_fd, relative_path)?")
            && snippet.contains("fs_safety::walk_to_parent(root_fd, relative_path)")
            && snippet.contains("fs_safety::stat_at_nofollow(&parent_fd, Path::new(entry_name))")
            && snippet.contains("OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW"),
        "scheduled audit must stay on fd-relative NOFOLLOW reads instead of path-based hashing"
    );
    assert!(
        !snippet.contains("file_state::stat_file(&absolute_path)")
            && !snippet.contains("file_state::hash_file(&absolute_path)"),
        "scheduled audit must not fall back to path-based stat/hash reads that follow symlinks"
    );
    assert!(
        snippet.contains("scheduled_full_hash_audit_missing_paths_marked_dirty")
            && snippet.contains("SET needs_full_sync = 1")
            && !snippet.contains("remaining_missing: plan.diff.missing.clone()"),
        "scheduled audit must mark the collection dirty on missing paths instead of deleting pages inline"
    );
}

#[test]
fn capture_phase1_drift_source_refuses_remap_on_material_changes() {
    let source = production_reconciler_source();
    let start = source.find("fn capture_phase1_drift(").unwrap();
    let end = source[start..]
        .find("fn run_phase2_stability_check")
        .map(|offset| start + offset)
        .unwrap();
    let snippet = &source[start..end];

    assert!(
        snippet.contains("RestoreRemapOperation::Remap if summary.has_material_changes()")
            && snippet.contains("ERROR: remap_drift_refused")
            && snippet.contains("ReconcileError::RemapDriftConflictError"),
        "Phase 1 remap capture must fail closed when old-root drift would otherwise be lost"
    );
}

#[test]
fn capture_phase1_drift_source_logs_restore_capture_without_refusal() {
    let source = production_reconciler_source();
    let start = source.find("fn capture_phase1_drift(").unwrap();
    let end = source[start..]
        .find("fn run_phase2_stability_check")
        .map(|offset| start + offset)
        .unwrap();
    let snippet = &source[start..end];

    assert!(
        snippet.contains("RestoreRemapOperation::Restore if summary.has_material_changes()")
            && snippet.contains("WARN: restore_drift_captured"),
        "Phase 1 restore capture must record the adopted drift without turning it into a remap-style refusal"
    );
}

#[test]
fn authorize_full_hash_source_requires_active_lease_for_remap_modes() {
    let source = production_reconciler_source();
    let start = source.find("fn authorize_full_hash_reconcile(").unwrap();
    let end = source[start..]
        .find("fn require_persisted_full_hash_owner_match(")
        .map(|offset| start + offset)
        .unwrap();
    let snippet = &source[start..end];

    assert!(
        snippet.contains("FullHashReconcileMode::RemapRoot,")
            && snippet.contains("FullHashReconcileMode::RemapDriftCapture,")
            && snippet.contains("FullHashReconcileAuthorization::ActiveLease { .. }"),
        "both remap full-hash modes must stay bound to the active owner lease"
    );
}
