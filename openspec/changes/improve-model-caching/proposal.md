## Why

Users experience unnecessary re-downloads of large language models (e.g., phi-3.5-mini at 6GB+) every time they enable extraction or start Quaid in online mode, even when the model is fully cached locally. This is caused by three interrelated failures in model lifecycle management: (1) incomplete downloads from network interruptions are not cleaned up, (2) stale temporary download directories persist indefinitely because cleanup only runs before download attempts, and (3) without a manifest file, Quaid cannot validate cached models and assumes they're incomplete.

## What Changes

- Add a `quaid cache clean` command to manually remove incomplete/stale model caches
- Improve error handling to guarantee temporary download directories are always cleaned on failures
- Add better logging and progress reporting for model downloads so interruptions are visible
- Refactor stale directory scavenging to be more aggressive and run on-demand
- Add cache validation that creates/updates manifest files for already-cached models
- Document model caching behavior and troubleshooting in operator guide

## Capabilities

### New Capabilities

- `cache-cleanup`: Explicit command (`quaid cache clean`) to remove stale/incomplete model caches with safety checks and verbose reporting
- `cache-validation`: Automatic cache integrity checks and manifest generation for existing cached models without requiring re-download
- `download-resilience`: Guaranteed cleanup of temporary directories on any download failure to prevent orphaned incomplete caches
- `download-observability`: Enhanced logging and progress reporting for model downloads, including network errors and timeouts

### Modified Capabilities

- `model-lifecycle`: Existing model download/cache system now includes robust error handling, explicit cleanup, and better observability

## Impact

- Affected code: `src/core/conversation/model_lifecycle.rs`, `src/commands/` (new cache command)
- Affected APIs: New CLI command `quaid cache clean`
- Dependencies: No new external dependencies
- User-facing: Reduces model download time by 2-4x for users with stale caches; adds safety mechanism to prevent cache corruption from interrupted downloads
- Telemetry: Can now track download failure rates and incomplete cache incidents
