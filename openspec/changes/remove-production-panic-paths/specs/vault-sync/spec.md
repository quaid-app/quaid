## ADDED Requirements

### Requirement: `finalize_pending_restore` is total and never panics on missing pending-restore state

`core::vault_sync::finalize_pending_restore` SHALL produce a `Result<FinalizeOutcome, VaultSyncError>` over every reachable combination of `(collection.state, collection.pending_root_path)` without invoking `panic!`, `unwrap`, or `expect` on the `pending_root_path` field. When `pending_root_path` is `None`, the function SHALL return `FinalizeOutcome::OrphanRecovered` if `state == CollectionState::Restoring` (after running `revert_orphan_restore_state`) and SHALL return `FinalizeOutcome::NoPendingWork` otherwise. The decision SHALL be expressed as a structurally-enforced match (e.g. `let Some(...) else { ... }`) rather than as a runtime guard followed by an unwrap, so that future refactors cannot silently reintroduce a panic path on the same field.

#### Scenario: Pending root path is NULL with state = `Created`

- **WHEN** `finalize_pending_restore` is called against a collection row whose `pending_root_path` is `NULL` and whose `state` is not `Restoring`
- **THEN** the function returns `Ok(FinalizeOutcome::NoPendingWork)` without panicking and without mutating the collection row

#### Scenario: Pending root path is NULL with state = `Restoring`

- **WHEN** `finalize_pending_restore` is called against a collection row whose `pending_root_path` is `NULL` and whose `state` is `Restoring`
- **THEN** the function calls `revert_orphan_restore_state`, returns `Ok(FinalizeOutcome::OrphanRecovered)`, and does not panic

#### Scenario: Pending root path present and exists on disk

- **WHEN** `finalize_pending_restore` is called against a collection row whose `pending_root_path` is `Some(path)` and `path` exists on disk
- **THEN** the function proceeds to manifest validation as before (no behavior change for the non-NULL path)
