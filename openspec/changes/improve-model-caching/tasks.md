## 1. Download Resilience - Error Handling

- [ ] 1.1 Add guaranteed cleanup wrapper for download operations
  - Implement try-finally pattern in `download_model_to_cache()` that removes temp_dir on all error paths
  - Ensure cleanup happens before returning error to caller
  
- [ ] 1.2 Improve error messages with actionable recovery instructions
  - Update `ModelLifecycleError` variants to include next steps (e.g., "Run `quaid cache clean`")
  - Ensure all error paths provide context: file name, expected vs. actual, paths affected

- [ ] 1.3 Add protection against concurrent download cleanup races
  - Implement check for `.downloading` lock files before cleanup
  - Prevent removing in-progress downloads from other Quaid instances
  - Document single-writer constraint

## 2. Download Observability - Logging

- [ ] 2.1 Add trace-level logging throughout download lifecycle
  - Log at download start: alias, repo_id, revision, file count, total size
  - Log per-file: URL, size, start time, hash type
  - Log per-file completion: actual hash, time elapsed, speed
  - Log cleanup operations: which files removed, why

- [ ] 2.2 Integrate error logging with existing ProgressReporter
  - Add methods or logging hooks for error reporting without changing interface
  - Ensure errors are logged at WARN level minimum (visible without RUST_LOG)
  - Ensure success completion is logged at INFO level

- [ ] 2.3 Add cache validation logging
  - Log manifest validation results (found/not found, valid/invalid)
  - Log file existence and hash verification for each file
  - Log when manifest is auto-generated vs. already present

## 3. Cache Validation - Manifest Generation

- [ ] 3.1 Implement lazy manifest generation for existing caches
  - Create function `ensure_cache_manifest(cache_dir, alias)` 
  - Scan cache_dir for all expected files based on alias
  - Compute SHA-256 for each file
  - Create manifest.json if all files present with correct hashes
  - Return error if files missing or hashes mismatch

- [ ] 3.2 Add manifest version field for forward compatibility
  - Update `CacheManifest` struct to include `manifest_version: u32`
  - Set to 1 for initial version
  - Add migration logic if version changes in future

- [ ] 3.3 Call manifest validation before model load
  - Update `load_model_from_local_cache()` to call `ensure_cache_manifest()`
  - Ensure this happens automatically without user intervention
  - Log trace-level message when manifest is auto-generated

## 4. Cache Cleanup Command - CLI

- [ ] 4.1 Create new `src/commands/cache.rs` command module
  - Add subcommands: `clean`, `status`
  - Implement `--list`, `--all`, `--force` flags for clean
  - Implement `--verbose` flag for status
  - Wire into CLI dispatcher in `main.rs`

- [ ] 4.2 Implement `quaid cache clean --list` subcommand
  - Scan ~/.quaid/models/ for temp directories (`.{cache_key}-download-*` pattern)
  - Identify incomplete caches (missing manifest or failed validation)
  - Display path, size, age, status for each
  - Calculate total disk space would be freed

- [ ] 4.3 Implement `quaid cache clean --all` with user confirmation
  - Prompt user with summary: "This will remove N directories, freeing X GB"
  - Support `--force` flag to skip confirmation
  - Remove directories one by one, tracking failures
  - Report results: successful removals, failures, total freed

- [ ] 4.4 Implement `quaid cache clean <alias>` for specific models
  - Remove only caches related to specified alias
  - Support both temp directories and complete caches with `--force`
  - Validate alias exists before offering to remove

- [ ] 4.5 Implement `quaid cache status` subcommand
  - List all cached models with columns: Alias | Status | Files | Size | Manifest | Modified
  - Show status: ✓ complete, ⚠️  incomplete, ❌ corrupted
  - Support `--verbose` to show file-by-file breakdown with SHA-256 hashes

## 5. Stale Directory Scavenging - Refactoring

- [ ] 5.1 Refactor scavenge_stale_download_dirs() for reusability
  - Extract core logic into reusable function that returns list of stale directories
  - Keep existing pre-download scavenging calling the refactored version
  - Make available to CLI command module for on-demand cleanup

- [ ] 5.2 Make TTL configurable via environment variable
  - Add `QUAID_STALE_CACHE_TTL_SECS` environment variable
  - Default to 6 hours (21600 seconds) if not set
  - Allow override for testing and custom deployments

## 6. Integration Tests

- [ ] 6.1 Add integration test for incomplete download recovery
  - Simulate download interruption (mock HTTP stream cutoff)
  - Verify temp dir cleanup happens
  - Verify subsequent download starts fresh without errors
  - Test: incomplete cache → cache clean → fresh download succeeds

- [ ] 6.2 Add integration test for manifest generation
  - Create mock cache directory with all files but no manifest
  - Call ensure_cache_manifest()
  - Verify manifest is created with correct structure
  - Verify file hashes in manifest match actual files

- [ ] 6.3 Add integration test for cache cleanup command
  - Create temp directories and incomplete caches
  - Run `quaid cache clean --list` and verify output
  - Run `quaid cache clean --all --force` and verify cleanup
  - Verify final status is empty

- [ ] 6.4 Add integration test for concurrent safety
  - Simulate two Quaid instances attempting to download same model
  - Verify only one succeeds and final cache is uncorrupted
  - Verify no duplicate files in cache directory

## 7. Documentation

- [ ] 7.1 Update operator guide with model caching troubleshooting section
  - Explain common failure scenarios and symptoms
  - Document recovery procedures (cache clean, re-download)
  - Explain QUAID_MODEL_CACHE_DIR and cache TTL configuration
  - Add examples: inspecting cache status, manual cleanup

- [ ] 7.2 Update CLI help text for new cache commands
  - `quaid cache --help` explains all subcommands with examples
  - `quaid cache clean --help` documents flags and safety behavior
  - `quaid cache status --help` explains output format and verbose mode

- [ ] 7.3 Add migration notes for users with stale caches
  - Explain that old incomplete caches will be automatically cleaned
  - Document how to manually recover existing complete caches via cache clean/status
  - Provide script to identify large stale caches before update

## 8. Testing and Validation

- [ ] 8.1 Run full test suite and verify no regressions
  - Existing model lifecycle tests pass
  - New tests for resilience, validation, cleanup pass
  - Integration tests pass in single and multi-instance scenarios

- [ ] 8.2 Manual testing with real models
  - Test with phi-3.5-mini: full download, interruption recovery, cache hit
  - Test with gemma-3-1b: verify manifest generation for existing cache
  - Test with network throttling/timeout scenarios

- [ ] 8.3 Verify error messages and logging are clear
  - Simulate various failure modes and verify error output
  - Verify logs at TRACE level capture sufficient detail for debugging
  - Verify users can recover without contacting support

- [ ] 8.4 Performance verification
  - Verify cache validation doesn't add significant latency (<100ms for typical cache)
  - Verify cleanup command completes in <30 seconds for typical cache directory
  - Verify logging doesn't noticeably slow down downloads

## 9. Code Review and Refinement

- [ ] 9.1 Peer review of error handling changes
  - Ensure all error paths are covered by cleanup
  - Verify no new resource leaks introduced
  - Check that error messages are user-friendly

- [ ] 9.2 Peer review of logging implementation
  - Verify logging levels are appropriate (TRACE for detail, INFO for success, WARN for non-fatal errors)
  - Check that sensitive information (paths, hashes) is not over-logged
  - Ensure performance impact is acceptable

- [ ] 9.3 Security review of cache cleanup command
  - Verify permissions are respected (don't remove caches user doesn't own)
  - Verify no path traversal vulnerabilities in cache directory scanning
  - Check that `--force` flag doesn't bypass important safety checks
