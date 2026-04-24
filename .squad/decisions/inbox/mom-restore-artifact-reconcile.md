# Decision: Quarantine Restore Artifact Reconciliation

**Author:** Mom  
**Date:** 2026-04-25  
**Commit:** 6a3d54c  
**Status:** LANDED

## Context

After Fry's restore artifact was rejected and Fry locked out, my prior commit
`e29d1d0` fixed the 5 blockers in `quarantine.rs` and `tests/quarantine_revision_fixes.rs`.
The worktree still contained uncommitted restore-related changes in 4 files:
`collection.rs`, `fs_safety.rs`, `vault_sync.rs`, `tests/collection_cli_truth.rs`.

These were leftover glue from the rejected Fry artifact — some required, one not.

## Audit Findings

### Required (kept and adopted)

| File | Change | Reason |
|------|--------|--------|
| `src/core/fs_safety.rs` | `linkat_parent_fd` (unix + stub) + test | Called by `install_tempfile_without_replace` in committed `quarantine.rs`; without it the crate does not compile |
| `src/core/vault_sync.rs` | `pub(crate)` on `ShortLivedLease` + `start_short_lived_owner_lease` | `quarantine.rs` calls `vault_sync::start_short_lived_owner_lease`; private visibility breaks the call site |
| `src/commands/collection.rs` | Restore arm routing + `quarantine_restore()` helper | CLI surface must route to the now-live `quarantine::restore_quarantined_page`; the bail-only arm is the rejected state |
| `tests/collection_cli_truth.rs` | Rewritten deferred test + new happy-path test | Tests must prove the live surface, not just that the bail fires |

### Dropped (Fry artifact)

| File | Change | Reason |
|------|--------|--------|
| `src/core/fs_safety.rs` | `walk_to_parent_create_dirs` (both variants + test + doc line) | Explicitly rejected in my prior commit (e29d1d0, Blocker 4). Absent parents must be *refused*, not silently recreated without a durable fsync chain. Fry's original implementation used this function; my revision switched to `walk_to_parent`. Keeping `walk_to_parent_create_dirs` in the module would imply it is available for use — it is not, and must not be. |

## Decision

**D-MR1: `walk_to_parent_create_dirs` is permanently excluded from the narrow
restore contract.** Any future caller that needs to create missing directories
must go through a separate, explicitly-audited path with its own atomicity and
fsync-chain guarantees. The current narrow restore contract requires the caller
to pre-create target directory structure; this is a documented gate, not a gap.

**D-MR2: The reconciled artifact is wholly Mom-authored.** The four files
committed in `6a3d54c` are owned by this revision. No Fry-sourced code
survives in the restore surface.

## End State

- `src/core/quarantine.rs` — restore implementation (e29d1d0, no change needed)
- `src/core/fs_safety.rs` — linkat_parent_fd only (walk_to_parent_create_dirs dropped)
- `src/core/vault_sync.rs` — crate-visible lease helpers
- `src/commands/collection.rs` — live restore routing
- `tests/quarantine_revision_fixes.rs` — unit-level gate proofs (e29d1d0)
- `tests/collection_cli_truth.rs` — CLI-level surface proofs

591 lib tests pass. 2 pre-existing Windows-only failures confirmed unrelated.
