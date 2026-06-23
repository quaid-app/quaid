## 1. Download Resilience - Cleanup Guards

- [x] 1.1 Add an extraction temp-directory cleanup guard
  - Guard owns the `.{cache_key}-download-{timestamp}-{uuid}` path
  - Guard removes the temp directory on `Drop` unless disarmed
  - Disarm only after successful rename or after another process has produced a verified final cache
  - Cover metadata/selection `?` early returns as well as file download errors

- [x] 1.2 Add an embedding temp-file cleanup guard
  - Guard owns each `{file}.download-{uuid}` path in `src/core/inference.rs`
  - Guard removes the temp file on network, write, hash, or rename failures
  - Disarm only after the temp file is successfully renamed or a valid destination already exists

- [x] 1.3 Add stale-download heartbeat handling
  - Write `.downloading` marker metadata for extraction temp directories
  - Refresh heartbeat during long downloads before TTL can expire
  - Skip cleanup for temp artifacts with recent heartbeat/mtime
  - Report cleanup failures instead of treating them as success

- [x] 1.4 Improve actionable error messages
  - Include alias, file name, URL when relevant, affected path, expected vs actual hash, and bytes received when known
  - Recommend `quaid model clean <alias> --force` plus `quaid model pull <alias>` where appropriate
  - Keep errors as `Result` values; do not add panics or test-only seams

## 2. Cache Inventory and Validation

- [x] 2.1 Add shared cache inventory helpers
  - Resolve cache root from `QUAID_MODEL_CACHE_DIR` or `~/.quaid/models`
  - Detect extraction SLM final caches and temp directories
  - Detect embedding model final caches and `*.download-*` temp files
  - Return structured status: family, alias/cache key, path, size, age, manifest state, validation state, cleanup eligibility

- [x] 2.2 Add manifest v1 while preserving legacy manifests
  - Add `manifest_version`, `created_at_unix`, `size_bytes`, and `modified_unix`
  - Read existing versionless manifests as legacy v0
  - Treat missing v1 fields as legacy, not corruption, when existing hashes validate
  - Upgrade legacy manifests only after successful full verification

- [x] 2.3 Implement safe lazy manifest generation
  - Generate manifests for curated extraction aliases from source pins
  - Generate manifests for known embedding aliases from pinned `ModelConfig` hashes
  - Generate manifests for custom models only after trusted metadata/checksums from a fresh pull
  - Fail closed for custom/unpinned existing caches without a manifest when trusted expectations are unavailable

- [x] 2.4 Split fast validation from full verification
  - Fast validation: manifest parse, path safety, file existence, size, and modified time
  - Full verification: SHA-256/git-blob digest recomputation
  - Run full verification after downloads, during manifest generation/upgrade, on fast-validation mismatch, and for `quaid model status --verify`
  - Do not re-hash multi-GB models on every normal cache hit

## 3. CLI - `quaid model status` and `quaid model clean`

- [x] 3.1 Extend `src/commands/model.rs`
  - Keep existing `quaid model pull <alias>` behavior
  - Add `status [alias] [--verbose] [--verify]`
  - Add `clean --list [alias]`
  - Add `clean --all [--force]`
  - Add `clean <alias> --force`

- [x] 3.2 Implement `quaid model status`
  - With no alias, list recognized caches with columns: Family | Alias/Key | Status | Files | Size | Manifest | Modified
  - With an alias, show matching cache details and validation result
  - With `--verbose`, show file-level paths, sizes, and manifest hashes
  - With `--verify`, perform full hash verification and report duration
  - Exit non-zero only for operational errors, not merely because a cache is missing/corrupted

- [x] 3.3 Implement `quaid model clean --list`
  - Preview stale temp directories, stale temp files, incomplete caches, and corrupted caches
  - Clearly distinguish "would remove" from "kept"
  - Show path, family, reason, age, and size
  - Calculate total disk space eligible for cleanup

- [x] 3.4 Implement `quaid model clean --all`
  - Prompt with a summary unless `--force` is supplied
  - Remove only stale/incomplete/corrupted artifacts, not complete verified caches
  - Continue after partial failures and report failed paths with reasons
  - Return non-zero when any requested removal fails

- [x] 3.5 Implement alias-specific cleanup
  - `quaid model clean <alias> --force` may remove the complete verified cache for that alias
  - Without `--force`, show the action that would be taken and require confirmation
  - Validate alias/cache matching before removing anything

## 4. Download Observability

- [x] 4.1 Improve `ConsoleProgressReporter`
  - Show alias, repo, revision, file count, and total bytes when known
  - Implement throttled `file_progress()` output with downloaded bytes, total bytes, speed, and ETA when known
  - Show per-file verification completion and final summary

- [x] 4.2 Keep observability dependency-free
  - Do not introduce `tracing`, `log`, env_logger, or `RUST_LOG` requirements in this change
  - Route user-visible diagnostics through progress output, command output, and returned errors

- [x] 4.3 Improve cleanup reporting
  - Report successful temp cleanup at verbose/status level
  - Report cleanup failures in normal error output when they affect recovery
  - Include recovery command suggestions

## 5. Tests

- [x] 5.1 Add extraction temp-directory cleanup tests
  - Metadata/file-selection error after temp dir creation leaves no temp dir
  - Network interruption leaves no temp dir
  - Hash mismatch leaves no temp dir
  - Rename race with existing verified cache returns cache hit and removes local temp

- [x] 5.2 Add embedding temp-file cleanup tests
  - GET failure after temp file creation removes temp file
  - Stream/write failure removes temp file
  - Hash mismatch removes temp file
  - Rename race with existing valid destination succeeds without corrupting cache

- [x] 5.3 Add manifest compatibility tests
  - Existing versionless manifest remains valid
  - Legacy manifest upgrades to v1 after full verification
  - Missing manifest for complete curated extraction cache is generated
  - Missing manifest for custom/unpinned cache fails closed when trusted expectations are unavailable

- [x] 5.4 Add CLI status/clean integration tests
  - `quaid model status` lists complete, missing, incomplete, and corrupted states
  - `quaid model status --verify` performs full hash validation
  - `quaid model clean --list` is dry-run only
  - `quaid model clean --all --force` removes eligible stale artifacts and keeps complete verified caches
  - `quaid model clean <alias> --force` removes that alias cache

- [x] 5.5 Add stale heartbeat/concurrency tests
  - Recent `.downloading` marker prevents cleanup
  - Expired heartbeat is eligible for cleanup
  - Two simulated downloads for the same alias leave one valid final cache and no extra temp paths

## 6. Documentation

- [x] 6.1 Update operator guide with model cache troubleshooting
  - Explain extraction SLM vs embedding model cache layouts
  - Document `QUAID_MODEL_CACHE_DIR`
  - Document `QUAID_STALE_MODEL_CACHE_TTL_SECS`
  - Show recovery examples using `quaid model status`, `quaid model clean`, and `quaid model pull`

- [x] 6.2 Update CLI help text
  - `quaid model --help` lists `pull`, `status`, and `clean`
  - `quaid model status --help` explains `--verbose` and `--verify`
  - `quaid model clean --help` explains dry-run, confirmation, and `--force`

- [x] 6.3 Add migration notes
  - Legacy versionless manifests are accepted
  - Complete source-verifiable caches can be repaired without re-download
  - Custom/unpinned caches without trusted metadata may require re-pull

## 7. Validation

- [x] 7.1 Run OpenSpec validation
  - `openspec validate improve-model-caching --strict`

- [x] 7.2 Run targeted test suites
  - `cargo test --no-default-features --features bundled,online-model,test-harness --test model_lifecycle`
  - Add and run new integration tests for model status/clean behavior

- [x] 7.3 Run lint/format gates
  - `cargo fmt --check`
  - `cargo clippy --all-targets --all-features --locked -- -D warnings`

- [x] 7.4 Performance check
  - Normal fast cache status should avoid full hashing and stay responsive for multi-GB caches
  - `status --verify` may take longer, but should report elapsed time and remain bounded by disk throughput
  - Cleanup should avoid recursively scanning unrelated directories outside the model cache root
