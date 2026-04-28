# Control-File Watcher Bypass

Use this when a filesystem watcher has a content-type filter (for example, "markdown files only") but the feature also depends on root-level control files like `.quaidignore`.

## Pattern

1. Identify control files that change runtime behavior without being normal content.
2. Detect them **before** the normal content filter drops the event.
3. Emit a dedicated watcher action (`IgnoreFileChanged`, config-reload event, etc.) instead of pretending they are ordinary content paths.
4. Route reload through the authoritative parser/reloader.
5. If reload fails, keep last-known-good state and **skip downstream reconcile/work** rather than running on stale control data.

## Why

Content filters are good at suppressing noise, but they are also a classic honesty trap: the watcher looks healthy while silently ignoring the file that changes the walk/query contract. For control files, "no event" is not neutral — it means the runtime is operating on stale policy.

## Quaid example

- `src/core/vault_sync.rs` should bypass the markdown-only filter for the root `.quaidignore`
- `src/core/ignore_patterns.rs::reload_patterns(...)` remains the sole authority for mirror refresh / parse-error handling
- Invalid `.quaidignore` must preserve the cached mirror and block reconcile until the control file is valid again
