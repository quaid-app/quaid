## ADDED Requirements

### Requirement: Temporary download artifacts are cleaned on failure
The system SHALL remove temporary download directories/files created by Quaid
when a download operation fails, unless the operating system refuses removal, in
which case the failure SHALL be reported and the artifact SHALL remain eligible
for later cleanup.

#### Scenario: Network error during multi-file extraction download
- **WHEN** an extraction model download fails due to a network error while downloading file 3 of 10
- **THEN** system removes the temporary extraction download directory
- **AND** reports any cleanup failure

#### Scenario: Network error during embedding download
- **WHEN** an embedding model file download fails after its temporary file is created
- **THEN** system removes the temporary `{file}.download-*` file
- **AND** keeps any previously completed valid files

#### Scenario: Hash verification fails
- **WHEN** file hash verification fails during download
- **THEN** system removes the temporary file or directory containing the failed file
- **AND** does not promote the failed artifact to the final cache path

#### Scenario: Disk full during download
- **WHEN** disk runs out of space during file write
- **THEN** system removes the temporary artifact it created
- **AND** reports the disk/write error clearly

#### Scenario: Rename failure
- **WHEN** moving a temporary artifact to the final cache location fails
- **THEN** system removes the temporary artifact if the final cache did not validate
- **AND** if another process already produced a valid final cache, system discards the local temporary artifact and treats the result as a cache hit

#### Scenario: Early return after temp creation
- **WHEN** code returns an error through `?` after creating a temporary download path but before the normal install block completes
- **THEN** the cleanup guard removes the temporary path before the error reaches the caller

### Requirement: Download state is recoverable
Users SHALL be able to safely retry failed downloads without manual filesystem
cleanup or cache corruption.

#### Scenario: Retry after network failure
- **WHEN** user runs `quaid model pull phi-3.5-mini` after a previous download failed
- **THEN** system starts from a clean temporary path
- **AND** stale partial artifacts from the failed attempt do not affect the retry

#### Scenario: Retry detects existing complete cache
- **WHEN** user runs `quaid model pull <alias>` and a complete valid cache already exists
- **THEN** system skips download and returns the verified cache immediately

#### Scenario: Concurrent downloads do not corrupt cache
- **WHEN** two Quaid instances attempt to download the same model simultaneously
- **THEN** at most one final cache is accepted
- **AND** the losing temporary artifact is removed or reported as a cleanup failure

### Requirement: Stale temporary artifacts do not accumulate indefinitely
The system SHALL identify and remove stale temporary download artifacts while
avoiding active downloads.

#### Scenario: Automatic stale cleanup before download
- **WHEN** a model download is initiated
- **THEN** system scavenges temporary artifacts older than the stale TTL before starting the new download

#### Scenario: Active temp artifact is skipped
- **WHEN** cleanup sees a temporary artifact with a recent `.downloading` heartbeat or recent modified time
- **THEN** system skips that artifact and continues cleanup

#### Scenario: Manual cleanup removes stale artifacts
- **WHEN** user runs `quaid model clean --all --force`
- **THEN** stale temporary artifacts are removed according to the cache-cleanup spec

### Requirement: Download failures include recovery instructions
Users SHALL receive clear error messages that explain what went wrong and how to
recover.

#### Scenario: Network timeout
- **WHEN** download times out
- **THEN** error message states the alias, file, timeout, and recommends retrying `quaid model pull <alias>`

#### Scenario: Hash mismatch
- **WHEN** file hash does not match expected value
- **THEN** error message states expected and actual digest values and recommends `quaid model clean <alias> --force` before retry

#### Scenario: Permission denied
- **WHEN** system cannot write to cache directory
- **THEN** error message states the path and recommends checking permissions or setting `QUAID_MODEL_CACHE_DIR`
