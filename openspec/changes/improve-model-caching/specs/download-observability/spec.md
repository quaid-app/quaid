## ADDED Requirements

### Requirement: Download progress is visible and granular
Users SHALL see detailed progress information for model downloads including per-file status and overall progress.

#### Scenario: Multi-file download shows progress
- **WHEN** downloading a multi-file model (e.g., 10 files)
- **THEN** system reports: "Downloading phi-3.5-mini [3/10] model-00001-of-00002.safetensors (5.0 GB / 6.1 GB)"

#### Scenario: Progress includes download speed
- **WHEN** a file is being downloaded
- **THEN** progress line includes estimated time remaining: "5.0 GB / 6.1 GB (2.1 MB/s, ~15m remaining)"

#### Scenario: Completion shows verification
- **WHEN** download completes for a file
- **THEN** status updates to show hash verification: "✓ model-00001-of-00002.safetensors (SHA-256: c5214cd...)"

#### Scenario: Summary on completion
- **WHEN** entire model download succeeds
- **THEN** system prints: "Downloaded phi-3.5-mini (10 files, 6.1 GB total) in 18m 42s"

### Requirement: Download failures are logged clearly
Users SHALL receive informative error logs that help diagnose download problems without requiring debug mode.

#### Scenario: Network error includes URL and details
- **WHEN** GET request fails
- **THEN** log includes: "GET https://huggingface.co/.../model-00001.safetensors failed: Connection timeout after 300s"

#### Scenario: Partial download doesn't hide incomplete state
- **WHEN** download stops mid-file
- **THEN** log includes: "Interrupted after 4.2 GB of 5.0 GB for model-00001.safetensors. Retrying will start fresh."

#### Scenario: Cache validation failure is logged
- **WHEN** manifest validation fails
- **THEN** log includes: "Cache validation failed for phi-3.5-mini: file tokenizer.json missing (expected in manifest)"

#### Scenario: Cleanup failure is reported
- **WHEN** temp directory cleanup fails
- **THEN** log includes: "Warning: Failed to remove stale cache at {path}: Permission denied"

### Requirement: Trace-level logging captures implementation details
Developers SHALL be able to enable trace logging to understand download internals for debugging.

#### Scenario: Trace logging for hash computation
- **WHEN** RUST_LOG=trace is set
- **THEN** logs include: "Computing SHA-256 for {path}... (chunk 1/128, 65KB)"

#### Scenario: Trace logging for manifest operations
- **WHEN** RUST_LOG=trace is set
- **THEN** logs include: "Writing manifest.json with 10 files, timestamp {ts}"

#### Scenario: Trace logging for cache scavenging
- **WHEN** RUST_LOG=trace is set
- **THEN** logs include: "Scavenging stale download dirs: found {N} candidates, {M} older than 6h, removing {K}"

### Requirement: Download observability integrates with existing progress reporter
Progress and errors SHALL use the existing `ProgressReporter` interface without requiring new dependencies.

#### Scenario: Progress updates don't require new interfaces
- **WHEN** downloading a model in any mode
- **THEN** implementation uses existing `ProgressReporter::file_started()`, `file_progress()`, `file_finished()` methods

#### Scenario: Errors flow through standard channels
- **WHEN** a download error occurs
- **THEN** error is returned as `ModelLifecycleError` and logged via tracing (existing infrastructure)

### Requirement: Cache status command provides observability
Users SHALL be able to inspect cache health and debug issues without technical knowledge.

#### Scenario: Cache status shows validation results
- **WHEN** user runs `quaid cache status`
- **THEN** output includes for each cache: "Status | Files | Size | Manifest | Last Modified"

#### Scenario: Cache status identifies problems
- **WHEN** a cache has issues
- **THEN** status output highlights: ❌ incomplete, ⚠️  no manifest, ⚠️  hash mismatch
