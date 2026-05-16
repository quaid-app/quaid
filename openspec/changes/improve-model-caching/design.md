## Context

### Current State

Quaid has two online model cache paths under `QUAID_MODEL_CACHE_DIR` or
`~/.quaid/models`:

- Extraction SLM cache: `src/core/conversation/model_lifecycle.rs`
  downloads Hugging Face model files into a temporary directory named
  `.{cache_key}-download-{timestamp}-{uuid}`, writes `manifest.json`, verifies
  it, then renames the temporary directory to the final cache directory.
- Embedding model cache: `src/core/inference.rs` downloads
  `config.json`, `tokenizer.json`, and `model.safetensors` directly into the
  final cache directory using per-file temporary names like
  `{file}.download-{uuid}`. It verifies pinned hashes for known embedding
  models but has no manifest and no shared cleanup/status surface.

The extraction path already cleans many failures, but cleanup is manual and can
be bypassed by Rust early returns before `install_result` is handled. The
embedding path can leave per-file temporary downloads when network, write, or
hash verification fails.

### The Problem

1. **Leaked partial downloads**: Network, disk, hash, metadata, or early-return
   errors can leave temporary directories/files behind.
2. **Legacy complete caches are treated as invalid**: Extraction caches without
   a manifest are removed and re-downloaded even when files are complete and
   source-verifiable.
3. **No unified cache inspection**: Users can pull a model but cannot ask Quaid
   which model caches are complete, incomplete, corrupted, or stale.
4. **Ambiguous progress/errors**: Download progress exists, but failures need
   more actionable context and recovery instructions.

### Constraints

- No new external dependencies.
- Must preserve existing cache contents and read legacy versionless manifests.
- Must work in airgapped builds; network repair is only available in the online
  build.
- Must not silently accept a cache unless the expected file set and digests are
  known from source pins, embedding model pins, an existing manifest, or fresh
  download metadata.
- Must tolerate concurrent Quaid processes without corrupting the final cache.

## Goals / Non-Goals

**Goals:**

- Remove partial extraction temp directories and embedding temp files on all
  download failure paths controlled by Quaid.
- Add `quaid model status` and `quaid model clean` to inspect and clean model
  caches without opening `memory.db`.
- Safely generate or upgrade manifests for complete, source-verifiable caches.
- Keep normal cache-hit validation fast while preserving an explicit full hash
  verification path.
- Make download failures and cleanup outcomes understandable from CLI output.

**Non-Goals:**

- Change model download sources or Hugging Face mirror strategy.
- Change model alias resolution semantics.
- Add remote telemetry or metrics collection.
- Add a distributed/shared model registry.
- Automatically remove complete verified caches by age.

## Decisions

### D1: Scope-Bound Cleanup Guards

**Decision**: Use small RAII cleanup guards for temporary download paths:

- Extraction SLM downloads use a temp-directory guard that removes the temp
  directory on `Drop` unless explicitly disarmed after a successful rename or
  after another process wins the race and leaves a valid final cache.
- Embedding downloads use a temp-file guard that removes `{file}.download-*` on
  `Drop` unless the file was successfully renamed or another process already
  produced a valid destination.

**Rationale**: Rust has no `try/finally`. A manual cleanup block after the
download call is easy to bypass with `?`, especially around metadata selection
or hash verification. A guard keeps idiomatic `?` propagation while making
cleanup the default.

**Alternatives Considered**:

- Manual "try-finally" cleanup blocks: rejected because current code already
  demonstrates how early returns can bypass them.
- Background cleanup thread: rejected because this is a CLI/runtime library and
  deferred cleanup is harder to test.
- RAII guard: selected because it is local, dependency-free, and idiomatic Rust.

### D2: Reuse the `quaid model` Command Surface

**Decision**: Extend `src/commands/model.rs` instead of adding a new top-level
`quaid cache` command.

**Subcommands**:

- `quaid model pull <alias>`: existing extraction SLM pull behavior.
- `quaid model status [alias] [--verbose] [--verify]`: show cache health.
- `quaid model clean --list [alias]`: dry-run cleanup preview.
- `quaid model clean --all [--force]`: remove stale/incomplete model cache
  artifacts, never complete verified caches.
- `quaid model clean <alias> --force`: remove artifacts associated with a
  specific model alias, including the complete cache for that alias.

**Rationale**: The CLI already says `quaid model` manages cached local models,
and existing specs/documentation already reference `quaid model pull`. Adding a
separate top-level `cache` group would split one workflow across two surfaces.

### D3: Cache Inventory Covers Both Layouts

**Decision**: Add shared inventory helpers that scan the model cache root and
classify entries by cache family and health.

The scanner SHALL detect:

- Extraction temp directories matching `.{cache_key}-download-*`.
- Embedding temp files matching `*.download-*` inside known embedding cache
  directories.
- Complete cache directories with valid manifests or known embedding files.
- Incomplete directories with missing required files.
- Corrupted caches with hash or manifest mismatches.

Normal `clean --all` removes only stale temp artifacts and incomplete/corrupted
cache directories. Complete verified caches are shown but kept unless the user
targets a specific alias with `--force`.

### D4: Manifest Compatibility and Safe Repair

**Decision**: Introduce a versioned manifest format while accepting legacy
versionless manifests.

New manifests SHALL include:

- `manifest_version: 1`
- `requested_alias`
- `repo_id`
- `revision`
- `created_at_unix`
- file entries with `path`, `sha256`, `size_bytes`, `modified_unix`, and
  `verified_from_source`

Legacy manifests without `manifest_version`, `created_at_unix`,
`size_bytes`, or `modified_unix` SHALL remain valid if their existing fields and
hashes validate. When a legacy manifest is verified, Quaid MAY upgrade it to v1.

Lazy manifest generation is allowed only when the expected file list and
expected digests are known:

- Curated extraction aliases from source pins.
- Known embedding models from `ModelConfig` pinned hashes.
- Freshly downloaded custom models where metadata/checksums were obtained
  during that pull.

Custom/unpinned model caches without a manifest SHALL fail closed unless Quaid
can validate them against trusted metadata. Quaid SHALL NOT generate a manifest
by hashing whatever files happen to be present, because that can mark a partial
cache as complete.

### D5: Fast Validation With Explicit Full Verification

**Decision**: Separate fast cache-hit validation from full hash verification.

Fast validation checks manifest structure, path safety, file existence, file
size, and file modified time. Full verification recomputes SHA-256/git-blob
digests and is required after downloads, during manifest generation/upgrade, and
when the user runs `quaid model status --verify`.

If fast validation detects a size or mtime mismatch, the cache becomes
suspect and Quaid runs full verification before accepting it. If full
verification fails, the cache is corrupted and model load fails closed.

**Rationale**: Re-hashing multi-gigabyte models on every cache hit or status
command can be slower than the command the user is trying to run. The manifest
still protects against silent corruption when files change.

### D6: Progress and Error Observability Without New Logging Dependencies

**Decision**: Use the existing `ProgressReporter` and returned error messages
for user-visible observability. Do not add `tracing`, `log`, or `RUST_LOG`
requirements in this change.

The console reporter should show:

- Planned alias, repo, revision, file count, and total bytes when known.
- Per-file start, throttled progress, speed, and ETA when byte totals are known.
- Per-file hash verification completion.
- Final download summary.

Errors should include the model alias, file name, URL when relevant, bytes
received when known, expected vs actual hash for integrity failures, affected
paths, and the recommended recovery command.

### D7: Stale Cleanup Uses TTL Plus Heartbeat

**Decision**: Keep the six-hour default stale TTL but make it configurable via
`QUAID_STALE_MODEL_CACHE_TTL_SECS`. Downloads write or refresh a lightweight
`.downloading` marker in temp directories and update it during progress. Cleanup
skips temp artifacts with a recent marker, even if the directory timestamp is
older than the TTL.

If cleanup cannot remove a path because the OS reports it is in use or access is
denied, Quaid reports the failure and leaves the path for a later cleanup
attempt. It does not claim success.

## Risks / Trade-offs

| Risk | Mitigation |
|------|------------|
| Cleanup removes an active slow download | Skip artifacts with a recent `.downloading` heartbeat; never remove recent temp paths by default. |
| Manifest generation accidentally blesses a partial cache | Generate manifests only from trusted expected file lists and expected digests. |
| Full hash validation is expensive | Use fast validation for normal cache hits and require full validation for repair, pull completion, mismatch suspicion, and `--verify`. |
| `model clean --all` removes data users expected to keep | `--all` removes stale/incomplete artifacts only; complete verified caches require alias-specific `--force`. |
| Existing legacy manifests fail after adding version fields | Read versionless manifests as legacy v0 and upgrade only after successful verification. |
| Two processes download the same model | Final rename remains race-safe: if another process creates a verified final cache first, discard local temp and return cache hit. |

## Migration Plan

1. Add cleanup guards and improved error/progress context for extraction SLM and
   embedding downloads.
2. Add manifest v1 structs/readers while accepting legacy manifests.
3. Add cache inventory, status, and cleanup helpers.
4. Add `quaid model status` and `quaid model clean`.
5. Update docs and migration notes.

No SQLite migration is required. Old cache directories remain valid if they can
be verified.

## Resolved Questions

1. **Should this add telemetry?** No. This change keeps observability local via
   status output, progress output, and error messages.
2. **Should cleanup repair manifests?** No. Status may validate and repair when
   safe; cleanup removes stale/incomplete artifacts.
3. **Should complete caches expire by age?** No. Only temp artifacts use TTL.
   Complete verified caches are removed only when explicitly targeted.
