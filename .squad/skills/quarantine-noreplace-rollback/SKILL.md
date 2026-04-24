# Quarantine No-Replace Rollback Proof

Use this when a vault-byte restore/install flow must prove two things at once:

1. the final install step does **not** replace a target that appears after any earlier absence check
2. any rollback unlink is followed by a parent-directory `fsync` before returning

## Pattern

- Create the staged bytes as a tempfile in the **same directory** as the final target.
- Do the final install with a no-replace primitive (`linkat` for same-dir files works well) instead of a plain rename after a pre-check.
- Remove the tempfile and `fsync` the parent directory **before** DB success is exposed.
- If any post-install step fails, unlink the installed target and `fsync` the parent before returning.

## Proof seam

- Add a tiny env-driven pause hook after the early target-absence check so an integration test can create a competing target and prove the install step still fails with the competing bytes preserved.
- Add a tiny env-driven trace hook that records `unlink:<name>` and `fsync-after-unlink:<name>` so rollback tests can assert the exact cleanup ordering without mocking the filesystem.

## Extended pattern: pre-install tempfile failure

If `write_all`/`sync_all` fail after tempfile creation, clean up the tempfile
**before** returning the error. Use the same `cleanup_tempfile` helper used on
install failure. If cleanup itself fails, return the cleanup error (honest about
worst-case state). Never leave orphaned tempfiles when the write path errors.

## Extended pattern: post-install work failure

Any work done **after** `linkat` installs the target (e.g. `parse_restored_page`,
DB updates, link resolution) must be wrapped with explicit rollback on failure.
Use `rollback_target_entry` (or equivalent) to unlink the installed target and
fsync the parent before returning the error. Callers must never see a half-installed
vault state.

## Extended pattern: absent parent refusal

Prefer `walk_to_parent` (no-create) over `walk_to_parent_create_dirs` for restore
paths. Absent parents mean something went wrong upstream; surface that clearly rather
than silently recreating directory trees without a durable fsync chain. Reserve the
create-dirs variant for paths that explicitly own directory provisioning.

## Good fit in GigaBrain

- `src/core/quarantine.rs` restore-style reactivation paths
- any future same-directory vault install path that must stay strict no-replace
