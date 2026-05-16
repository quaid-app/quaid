## MODIFIED Requirements

### Requirement: Download errors don't leave orphaned temporary directories
The system SHALL guarantee cleanup of temporary directories on all download failure paths.

**Previous behavior**: Temp directories could persist if downloads failed, especially on rename failures or concurrent operations.

**Updated behavior**: Every download operation has guaranteed cleanup via try-finally semantics. Temp directories are removed regardless of failure type.

#### Scenario: Guaranteed cleanup on any error
- **WHEN** a download fails for any reason (network, permissions, timeout, hash mismatch, disk full, etc.)
- **THEN** the temporary directory is removed and no residue remains

#### Scenario: Cleanup succeeds even if files are locked
- **WHEN** cleanup attempts to remove temporary files that are still in use
- **THEN** cleanup defers removal via OS-specific deferred delete (Windows) or marks for cleanup (Unix), rather than failing

#### Scenario: Rename failure doesn't leave ambiguous state
- **WHEN** temporary directory rename to final cache location fails
- **THEN** system verifies whether cache succeeded (if cache_dir exists and validates, keep it; if not, remove temp_dir)

### Requirement: Model caches can be validated without re-download
The system SHALL automatically generate manifest files for existing complete caches.

**Previous behavior**: If manifest.json was missing, model was assumed incomplete and required re-download.

**Updated behavior**: System validates files and auto-generates manifest if all files are present with correct hashes.

#### Scenario: Lazy manifest generation
- **WHEN** model cache exists but manifest.json is missing
- **THEN** system validates all files exist with correct SHA-256/git-blob hashes and creates manifest.json

#### Scenario: Validation failure prevents silent cache acceptance
- **WHEN** cache exists but file hashes don't match expected values
- **THEN** manifest generation fails and cache is marked as corrupted; re-download is required

### Requirement: Stale temporary directories are actively cleaned
The system SHALL remove old incomplete temporary download directories to prevent disk accumulation.

**Previous behavior**: Stale directories were only cleaned during the next download attempt via scavenge_stale_download_dirs().

**Updated behavior**: Users can manually trigger cleanup via `quaid cache clean`, and pre-download scavenging remains for automatic cleanup.

#### Scenario: Manual cache cleanup
- **WHEN** user runs `quaid cache clean --all`
- **THEN** system removes all temporary download directories older than 6 hours

#### Scenario: Pre-download scavenging still occurs
- **WHEN** model download is initiated
- **THEN** system scavenges and removes stale temp directories before starting download

### Requirement: Download state transitions are explicitly observable
The system SHALL provide clear logging of download lifecycle events at appropriate log levels.

**Previous behavior**: Download progress was reported via ProgressReporter but errors were not consistently logged.

**Updated behavior**: All significant events (start, file completion, error, cleanup, success) are logged with context.

#### Scenario: Download start is logged
- **WHEN** model download begins
- **THEN** system logs: "Starting model download: phi-3.5-mini from microsoft/Phi-3.5-mini-instruct (revision: main)"

#### Scenario: Download error includes context
- **WHEN** download fails
- **THEN** system logs: "Download failed after 45 minutes: {file} - {error}. Cleaning up temporary cache."

#### Scenario: Successful completion is logged
- **WHEN** download completes and cache is moved to final location
- **THEN** system logs: "Model successfully cached: phi-3.5-mini at {cache_path} (10 files, 6.1 GB)"
