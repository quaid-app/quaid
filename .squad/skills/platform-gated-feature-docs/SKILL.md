---
name: platform-gated-feature-docs
version: 1.0
author: amy
last_updated: 2026-04-24
---

# Platform-Gated Feature Documentation

Use this skill when a CLI command or feature is gated behind a platform check (`#[cfg(unix)]`, `ensure_unix_platform`, etc.). The goal is to prevent users on unsupported platforms from being silently surprised by runtime errors.

## When to apply

- A Rust source file calls `ensure_unix_platform(...)` or has `#[cfg(unix)]` guards on a public CLI subcommand.
- A feature description in docs makes no mention of platform requirements.
- PR review feedback notes that a docs example will fail on Windows/Linux/macOS because the public command boundary is more tightly gated than the backing helper.

## Pattern: adding a platform callout

Place the callout **immediately after** the feature description, before any subcommand examples:

```markdown
> **Unix only.** `gbrain <command>` requires a Unix platform (macOS or Linux).
> On Windows it returns `UnsupportedPlatformError`.
> [Optional: note what IS cross-platform — e.g. read/search CLI paths.]
```

### What to include

1. **The gated command** by exact name.
2. **The error users will see** on unsupported platforms (e.g. `UnsupportedPlatformError`).
3. **What still works** cross-platform — prevents over-blocking users who only need the portable subset.
4. **Deferred vs. missing** — verify whether only a sub-feature is gated or the whole public command is gated, then describe that exact boundary.

## Pattern: correcting "not yet implemented" notes

When a feature is described as "deferred" or "not yet implemented" but the code already has an implementation (even if narrow or platform-gated), the note is factually false and will appear in PR review.

Correct approach:
1. Check the implementation — `grep` for the function name in `src/`.
2. If it exists, identify any `#[cfg(unix)]` or `ensure_unix_platform` guards.
3. Replace "not yet implemented" with an accurate description of what IS implemented and what IS deferred.
4. Scope the deferred note to the specific sub-path still missing (e.g. "IPC/online-handshake path" vs. "all of restore").

## Anti-patterns

- Saying a feature is unavailable when it is available on a subset of platforms.
- Saying a feature is available without noting the platform constraint.
- Leaving "not yet implemented" notes after the implementation lands on a feature branch.
- Assuming only the internal watcher is gated when the public `serve` command is gated at the CLI entrypoint.

## Checklist before shipping platform-restricted features

- [ ] `grep -r "ensure_unix_platform\|#\[cfg(unix)\]" src/commands/` — identify all platform gates.
- [ ] For each gated command, confirm the docs callout exists in all surfaces: `docs/getting-started.md`, `website/src/content/docs/`, README.
- [ ] Confirm "deferred" notes are scoped only to the truly missing sub-path, not the whole feature.
- [ ] Confirm the MCP server config example (`gbrain serve`) carries the Unix-only note if the whole `serve` command is gated, not just the watcher internals.
