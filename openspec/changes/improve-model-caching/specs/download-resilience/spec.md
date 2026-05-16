## ADDED Requirements

### Requirement: Temporary directories are cleaned on download failure
The system SHALL guarantee that temporary download directories are removed when a download operation fails, preventing orphaned incomplete caches.

#### Scenario: Network error during multi-file download
- **WHEN** a download fails due to network error while downloading file 3 of 10
- **THEN** system removes the temporary directory completely, regardless of error type or location

#### Scenario: Hash verification fails
- **WHEN** file hash verification fails during download
- **THEN** system removes the incomplete file and the temporary directory, leaving no residue

#### Scenario: Disk full during download
- **WHEN** disk runs out of space during file write
- **THEN** system removes the temporary directory and reports error clearly

#### Scenario: Rename failure
- **WHEN** moving temporary directory to final cache location fails (permissions, already exists, etc.)
- **THEN** system removes the temporary directory if rename fails, but preserves cache if cache already completed

#### Scenario: Timeout on final file
- **WHEN** download times out on the last file of a multi-file model
- **THEN** system removes the temporary directory including partially-downloaded final file

### Requirement: Download state is recoverable
Users SHALL be able to safely retry failed downloads without manual cleanup or corruption.

#### Scenario: Retry after network failure
- **WHEN** user runs `quaid model pull phi-3.5-mini` after a previous download failed
- **THEN** system starts fresh download from beginning, with old temp directory already cleaned

#### Scenario: Retry detects existing complete cache
- **WHEN** user runs `quaid model pull` and a complete valid cache already exists
- **THEN** system skips download and returns cache immediately (cache hit)

#### Scenario: Concurrent downloads don't corrupt cache
- **WHEN** two Quaid instances attempt to download the same model simultaneously
- **THEN** at most one succeeds; the other waits or fails gracefully without corrupting shared cache

### Requirement: No orphaned temporary directories accumulate
The system SHALL prevent temporary download directories from accumulating indefinitely on disk.

#### Scenario: Automatic stale cleanup before download
- **WHEN** a model download is initiated
- **THEN** system scavenges and removes temporary directories older than 6 hours before starting download

#### Scenario: Cleanup handles already-in-use temp directories
- **WHEN** attempting to remove a stale temp directory that another process is still writing to
- **THEN** system skips that directory (doesn't delete in-progress downloads) and continues cleanup

#### Scenario: Manual cleanup via cache clean command
- **WHEN** user runs `quaid cache clean --all`
- **THEN** all temporary directories are identified and can be removed (see cache-cleanup spec)

### Requirement: Download failures are explicit and actionable
Users SHALL receive clear error messages that explain what went wrong and how to recover.

#### Scenario: Network timeout
- **WHEN** download times out
- **THEN** error message states: "Download timeout after 300 seconds for {file}. Check your network or retry with `quaid model pull {alias}`"

#### Scenario: Hash mismatch
- **WHEN** file hash doesn't match expected value
- **THEN** error message states: "Integrity check failed for {file}: expected SHA-256 {expected}, got {actual}. File may be corrupted. Run `quaid cache clean {alias}` and retry."

#### Scenario: Permission denied
- **WHEN** system can't write to cache directory
- **THEN** error message states: "Permission denied writing to cache at {path}. Check directory permissions or set QUAID_MODEL_CACHE_DIR environment variable."
