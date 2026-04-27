# Fry — Issue #81 watcher empty-root decision

- Decision: treat any `collections` row with `state='active'` and blank `root_path` as invalid legacy state during serve watcher bootstrap, and normalize it to `state='detached'` before watcher registration.
- Why: the crash surface happens before any filesystem work worth preserving; demoting the row is safer than attempting to watch an empty path or silently keeping an impossible active root around.
- Implementation seam: `src/core/vault_sync.rs::detach_active_collections_with_empty_root_path()` runs from watcher sync, logs `WARN: serve_detached_empty_root ...`, and leaves watcher selection gated on `trim(root_path) != ''`.
