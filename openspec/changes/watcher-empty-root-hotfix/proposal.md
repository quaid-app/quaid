# Watcher Empty-Root Hotfix

**Status:** complete  
**Issue:** #81 (beta feedback — watcher crash on empty root_path)  
**Branch:** fry/issue-81-empty-root-watcher  
**Lane:** hotfix — no new capability, pure correctness fix

## Problem

`quaid serve` crashes with `InvariantViolationError: failed to watch root` when any
collection is in `state='active'` with an empty `root_path`.

Root cause: `ensure_default_collection` in `db.rs` inserts the default collection with
`state='active'` and `root_path=''`. On first `quaid init`, before the user runs
`collection add`, this is the initial state of memory. `sync_collection_watchers`
queries all `state='active'` collections and calls `watcher.watch("")`, which fails.

This is an invariant violation in the schema: the codebase uses
`CASE WHEN root_path = '' THEN 'detached' ELSE 'active' END` in three places to
maintain this invariant, but the bootstrap insert violated it.

## Fix

Three changes:

1. **`src/core/db.rs`** — `ensure_default_collection` now inserts with `state='detached'`
   instead of `state='active'` when `root_path=''`. This is correct because a collection
   with no root path is by definition detached (not watching anything).

2. **`src/core/vault_sync.rs`** — `detach_active_collections_with_empty_root_path()` now
   demotes any legacy `state='active' AND trim(root_path) = ''` rows to `detached`
   before watcher registration. This repairs already-initialized brains without a
   migration and makes the behavior explicit in logs.

3. **`src/core/vault_sync.rs`** — `sync_collection_watchers` SQL adds
   `AND trim(root_path) != ''` as a defensive guard so watcher selection itself
   stays fail-closed even if another invalid row appears.

## Non-goals

- No schema migration required (the guard handles the old bad state in existing DBs).
- No change to the `default` collection's role as the catch-all for legacy pages.
- No change to the vault-sync-engine change scope — this is a bootstrap bug, not a
  watcher-core behavioral gap.

## Test

- `detach_active_collections_with_empty_root_path_normalizes_default_collection`
  in `src/core/vault_sync.rs`
- `sync_collection_watchers_skips_active_collection_with_empty_root_path` in
  `src/core/vault_sync.rs` (unix-only, matches the unix gate on the watcher)
