## Context

### Current State
Model lifecycle management is handled in `src/core/conversation/model_lifecycle.rs`. The system:
- Downloads models from HuggingFace to temporary directories (`.{cache_key}-download-{timestamp}-{uuid}`)
- Verifies SHA-256 hashes during download
- Renames temporary directories to final cache locations on success
- Creates `manifest.json` with file paths and hashes for validation
- Runs `scavenge_stale_download_dirs()` before each download to clean directories older than 6 hours
- Downloads timeout after 300 seconds per file, but entire multi-file downloads can take much longer

### The Problem
1. **Incomplete downloads**: Network interruptions during multi-file downloads leave partial caches in temp directories
2. **Orphaned temp dirs**: Scavenge only runs before downloads, so stale directories accumulate if users don't download frequently
3. **Silent failures**: No manifest validation means Quaid assumes incomplete caches are actually complete and skips re-download
4. **Limited observability**: Download progress is reported but failures are not clearly logged

### Constraints
- Must be backward compatible (don't break existing cache format)
- Must work in airgapped mode (embedded model builds)
- Must not require external commands or dependencies
- Must handle concurrent Quaid instances (single-writer guarantee via SQLite)

## Goals / Non-Goals

**Goals:**
- Eliminate orphaned incomplete model caches through guaranteed cleanup on download failures
- Enable manual cache cleanup via `quaid cache clean` command
- Add manifest validation for existing cached models so users can recover from stale caches without re-download
- Improve download failure observability with clear logging and error reporting
- Make stale directory scavenging more aggressive and on-demand

**Non-Goals:**
- Change model download sources or CDN strategy
- Modify model selection/alias resolution logic
- Add distributed cache (e.g., shared model registry)
- Implement model-specific garbage collection policies

## Decisions

### D1: Guaranteed Cleanup on Download Failure

**Decision**: Wrap all download operations in a try-finally pattern that unconditionally removes the temp directory on any error.

**Current State**: Error paths call `fs::remove_dir_all(&temp_dir)` manually, but may not catch all errors (e.g., during rename, after partial success).

**Change**: After download completes (successfully or with error), always attempt cleanup:
```rust
let result = download_files(&temp_dir);
match fs::rename(&temp_dir, &cache_dir) {
    Ok(_) => Ok(cache_dir),
    Err(e) if cache_dir.is_dir() && verify_cache_manifest(...).is_ok() => {
        let _ = fs::remove_dir_all(&temp_dir);  // Remove temp, cache won already
        Ok(cache_dir)
    }
    Err(e) => {
        let _ = fs::remove_dir_all(&temp_dir);  // Always clean temp on rename failure
        Err(...)
    }
}
```

**Rationale**: Ensures temp directories never accumulate from failures.

**Alternatives Considered**:
- Use guard types (RAII) - adds complexity to download code flow
- Schedule cleanup in background - deferred cleanup is harder to reason about
- ✓ Try-finally cleanup - simple, explicit, guaranteed

### D2: New `quaid cache clean` Command

**Decision**: Add a new CLI command under `src/commands/cache.rs` that lists and removes stale/incomplete model directories.

**Subcommands**:
- `quaid cache clean --list` - Show stale directories that would be removed (dry-run)
- `quaid cache clean --all` - Remove all stale directories
- `quaid cache clean <alias>` - Remove cache for specific model alias
- `quaid cache status` - Show cache health (completeness, manifest validity)

**Implementation**: 
- Reuse `scavenge_stale_download_dirs()` logic but make it on-demand
- Scan `~/.quaid/models/` and identify:
  - Temp directories: `.*-download-.*` pattern (always stale)
  - Incomplete caches: exist but missing manifest or failed validation
  - Aged caches: older than TTL (6 hours for temps, optional for full caches)
- Report what will be removed with sizes
- Require confirmation before deletion (unless `--force`)

**Rationale**: Gives users explicit control and visibility into cache health without waiting for the next download.

**Alternatives Considered**:
- Auto-cleanup on startup - disruptive, slow startup
- Keep stale cleanup before-download only - leaves manual intervention only option
- ✓ On-demand CLI command - explicit, safe, debuggable

### D3: Manifest Validation for Existing Caches

**Decision**: Add `cache-validation` capability that checks existing caches and auto-generates missing manifests.

**Flow**:
1. When cache is first used, check if `manifest.json` exists
2. If missing, compute SHA-256 for all cache files and create manifest
3. If exists but invalid, report error and recommend re-download

**Benefits**:
- Users with partially-downloaded caches (from previous incomplete downloads) can recover
- No re-download required if files are actually complete
- Manifest acts as proof of cache completeness

**Implementation**:
- New function `ensure_cache_manifest(cache_dir, alias)` called before model load
- Scans all files in cache_dir, computes hashes, validates against expected set for alias
- Creates manifest only if all files present and hashes match

**Alternatives Considered**:
- Require re-download if manifest missing - wasteful for users with complete caches
- ✓ Lazy manifest generation - recovers existing good caches automatically

### D4: Improved Logging and Observability

**Decision**: Add debug/trace logging throughout download lifecycle and model the error states clearly.

**Where**:
- Pre-download: log cache validation results, temp directory status
- During download: log each file start, progress checkpoints, errors
- Post-download: log manifest creation, rename success/failure, cleanup success/failure
- Error reporting: Include file size, download duration, hash mismatches

**Rationale**: Makes it easier to debug download issues and understand why a cache was invalidated.

**Implementation**: Use existing `ProgressReporter` trait but extend with more granular logging at trace level.

### D5: Stale Directory Scavenging Strategy

**Decision**: Keep pre-download scavenging but add optional on-demand cleanup via `quaid cache clean`.

**Rationale**:
- Pre-download scavenging is lightweight and automatic (wins for most users)
- On-demand cleanup via CLI is explicit and useful for operators and troubleshooting
- Don't aggressively clean on every command (too much FS I/O)
- 6-hour TTL is reasonable for temp directories

**Alternative**: Reduce TTL to 1 hour (more aggressive) - rejected as too aggressive for large models

## Risks / Trade-offs

| Risk | Mitigation |
|------|-----------|
| Cleanup removes in-progress downloads if two Quaid instances run simultaneously | Quaid has single-writer design; concurrent reads are OK. Document that only one instance should download at a time. Add check for `.downloading` lock files. |
| Manifest validation has false negatives (missing files but marked complete) | Verify manifest immediately after creation, reject if any file hashes mismatch. Add integration test for incomplete downloads. |
| `quaid cache clean` is too aggressive and removes user's intended caches | Add `--list` dry-run first, require `--force` for destructive operations, warn about non-stale caches. |
| User confusion about when to run `cache clean` | Add troubleshooting section to operator guide explaining download failure symptoms and recovery. |
| Download failures not immediately visible if user doesn't check logs | Improve progress reporter to surface errors to stdout/stderr even in non-verbose mode. |

## Migration Plan

1. **Phase 1 (PR)**: Add guaranteed cleanup on download errors + improved logging
2. **Phase 2 (PR)**: Add `cache-validation` capability (lazy manifest generation)
3. **Phase 3 (PR)**: Add `quaid cache clean` command with subcommands
4. **Deployment**: Changes are fully backward compatible; no data migration needed
5. **Rollback**: None required; old code path remains if cleanup fails

## Open Questions

1. Should we add metrics/telemetry for download failures and incomplete caches? (Suggested: yes, helps identify patterns)
2. Should `cache clean` also validate and repair manifests for existing caches, or only remove stale ones? (Suggested: separate `cache validate` subcommand)
3. What's the maximum age for a non-temp cache before it's considered stale? (Suggested: no auto-removal, only manual)
