#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Reconcile commit-batching: a bulk attach folds per-file apply actions into
//! ~256-file transactions instead of one commit per file, so the commit count
//! for a large attach is bounded by ceil(files / 256) rather than O(files).

#[path = "common/reconciler_fixtures.rs"]
mod common_reconciler_fixtures;

use common_reconciler_fixtures::*;
use quaid::core::reconciler::*;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tempfile::TempDir;

#[cfg(unix)]
#[test]
fn bulk_attach_batches_apply_commits() {
    let conn = open_test_db();
    let root = TempDir::new().unwrap();

    // 200 brand-new markdown files: each is a "new" page the reconciler must
    // reingest. Pre-batching this was ~200 commits; batched it is one
    // 256-file transaction.
    let file_count = 200usize;
    for index in 0..file_count {
        let body = format!(
            "---\nslug: notes/note-{index:04}\ntitle: Note {index}\ntype: concept\n---\nBody {index}.\n"
        );
        fs::write(root.path().join(format!("note-{index:04}.md")), body).unwrap();
    }
    let collection = insert_collection(&conn, root.path());

    // Count COMMITs via a commit hook. Returning false from the hook allows the
    // commit to proceed; we only tally.
    let commits = Arc::new(AtomicU64::new(0));
    let commits_for_hook = Arc::clone(&commits);
    conn.commit_hook(Some(move || {
        commits_for_hook.fetch_add(1, Ordering::SeqCst);
        false
    }));

    let stats = reconcile(&conn, &collection).unwrap();
    conn.commit_hook::<fn() -> bool>(None);

    assert_eq!(stats.new, file_count, "every file should ingest as new");

    let observed = commits.load(Ordering::SeqCst);
    // Generous ceiling: the apply path contributes ceil(200/256) = 1 batch
    // commit; the surrounding reconcile pass adds a handful of bookkeeping
    // commits (stat refresh, sync stamps). Far below the pre-batching ~200.
    assert!(
        observed < 20,
        "expected batched commits well under the per-file count, observed {observed} for {file_count} files"
    );
}
