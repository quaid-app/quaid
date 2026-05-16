## Why

Users can lose time and disk space when online model downloads fail or when a complete cache cannot be trusted. Extraction SLM caches already use temporary directories plus `manifest.json`, but some early-return paths can still leave temporary directories behind and legacy complete caches without manifests are discarded instead of repaired. Online embedding model caches use per-file temporary downloads and no manifest, so failures can leave `*.download-*` files in otherwise valid cache directories.

The fix needs to cover both cache layouts that live under `QUAID_MODEL_CACHE_DIR` / `~/.quaid/models`: extraction SLM caches managed by `src/core/conversation/model_lifecycle.rs`, and online embedding caches managed by `src/core/inference.rs`.

## What Changes

- Extend the existing `quaid model` command with `status` and `clean` subcommands for model cache inspection and cleanup
- Add scope-bound cleanup guards for temporary download directories/files so Rust `?` early returns cannot leak partial downloads
- Add cache inventory helpers that understand both extraction SLM temp directories and embedding temp files
- Add backward-compatible manifest validation/upgrade for complete caches, without trusting arbitrary partial file sets
- Improve user-facing progress and error reporting through the existing progress/error channels so interruptions are visible
- Refactor stale cache scavenging so it is reusable by automatic pre-download cleanup and manual `quaid model clean`
- Document model caching behavior and troubleshooting in operator guide

## Capabilities

### New Capabilities

- `cache-cleanup`: Explicit command (`quaid model clean`) to remove stale/incomplete model caches with safety checks and verbose reporting
- `cache-validation`: Automatic cache integrity checks plus safe manifest generation/upgrade for complete, source-verifiable cached models without requiring re-download
- `download-resilience`: Cleanup guards for temporary directories/files on download failures to prevent orphaned incomplete caches
- `download-observability`: Enhanced progress and error reporting for model downloads, including network errors, partial downloads, and cleanup failures

### Modified Capabilities

- `model-lifecycle`: Existing model download/cache systems now include robust error handling, explicit cleanup, cache status, and repairable manifests

## Impact

- Affected code: `src/core/conversation/model_lifecycle.rs`, `src/core/inference.rs`, `src/commands/model.rs`, docs and integration tests
- Affected APIs: New CLI subcommands under existing `quaid model`: `status` and `clean`
- Dependencies: No new external dependencies
- User-facing: Avoids unnecessary re-downloads for complete legacy caches, makes stale cache cleanup explicit, and gives actionable recovery steps after interrupted downloads
- Telemetry: No new telemetry. Operators get human-readable status output and clearer command errors
