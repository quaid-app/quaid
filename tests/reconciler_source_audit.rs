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

// PIN JUSTIFICATION: the safety pipeline's *internal* phase ordering (drift
// capture → stability proof → pre-destruction fence → fresh-connection dirty
// recheck) and the no-mount wrapper's closure wiring are fd/mount-verifier
// seams that cannot be observed from the public restore/remap API — the
// behavioural restore/remap tests (`tests/vault_sync_restore.rs`,
// `tests/vault_sync_remap.rs`) prove the *outcome* (drift refusal, new-root
// verification, fail-closed halts) but not the relative ordering of these
// private phases. These two source pins guard that ordering against a silent
// refactor that would reorder the fence after the dirty recheck.

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
        snippet.contains(
            "run_restore_remap_safety_pipeline_inner(conn, request, |_| Ok(()), || Ok(()))"
        ),
        "the no-mount wrapper must reuse the shared safety pipeline with a no-op mount verifier"
    );
}

#[test]
fn remap_safety_pipeline_source_keeps_phase_order_before_dirty_recheck() {
    let source = production_reconciler_source();
    let start = source
        .find("fn run_restore_remap_safety_pipeline_inner")
        .expect("shared restore/remap safety pipeline must remain present");
    let end = source[start..]
        .find("pub(crate) fn run_restore_remap_safety_pipeline_without_mount_check(")
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

// PIN JUSTIFICATION: this pins the *symlink-safety mechanism* — fd-relative
// NOFOLLOW reads (`OFlags::NOFOLLOW`, `stat_at_nofollow`, `walk_to_parent`)
// rather than path-based stat/hash that follows symlinks. The security
// property (a symlinked audit path is not followed) is impossible to assert
// from a unit test without racing a TOCTOU symlink swap against the live
// audit; the flag choice itself is the load-bearing invariant, so it stays a
// source pin per the established `tests/cli_put_source_invariants.rs` pattern.
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

// NOTE: the remap-drift-refusal source pin that used to live here was deleted.
// Its property — Phase 1 fails closed with `RemapDriftConflictError` when
// old-root drift would otherwise be silently lost — is now fully driven
// behaviourally by `remap_collection_refuses_phase1_drift_until_new_root_catches_up`
// in `tests/vault_sync_remap.rs`, which mutates the old root mid-remap and
// asserts the typed error plus the preserved drift in `raw_imports`.

// PIN JUSTIFICATION: the *restore* branch of Phase 1 (as opposed to remap)
// adopts old-root drift rather than refusing it, and the only externally
// observable signal of that decision is a `WARN: restore_drift_captured` log
// line (the page contents converge either way, so a behavioural assert cannot
// distinguish "captured" from "no drift"). This pin guards that the restore
// branch keeps capturing-without-refusing.
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

// PIN JUSTIFICATION: the authorization match arms bind both remap full-hash
// modes to an active owner lease. Driving the *negative* case behaviourally
// (a remap full-hash attempted without the active lease) would require
// fabricating an inconsistent owner-lease/state combination that the rest of
// the runtime refuses to produce, so the binding stays a source pin.
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
