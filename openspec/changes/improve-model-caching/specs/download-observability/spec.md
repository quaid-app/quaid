## ADDED Requirements

### Requirement: Download progress is visible and granular
Users SHALL see useful progress information for model downloads, including
per-file status and overall completion context when available.

#### Scenario: Multi-file download shows file progress
- **WHEN** downloading a multi-file model
- **THEN** system reports the alias, source repo, revision, file count, current file, and file index

#### Scenario: Progress includes byte counts and speed
- **WHEN** the server provides a content length while a file is being downloaded
- **THEN** progress output includes downloaded bytes, total bytes, current speed, and estimated time remaining

#### Scenario: Unknown-size progress still advances
- **WHEN** the server does not provide a content length
- **THEN** progress output still reports downloaded bytes for the current file

#### Scenario: Completion shows verification
- **WHEN** download completes for a file
- **THEN** status reports that the file passed integrity verification

#### Scenario: Summary on completion
- **WHEN** entire model download succeeds
- **THEN** system prints a summary with alias, file count, total bytes when known, elapsed time, and cache path

### Requirement: Download failures are explicit and actionable
Users SHALL receive informative errors that help diagnose download problems
without enabling a separate logging framework.

#### Scenario: Network error includes URL and details
- **WHEN** GET request fails
- **THEN** error output includes the URL, model alias, file name when known, and underlying network error

#### Scenario: Partial download includes byte count
- **WHEN** download stops mid-file after some bytes were received
- **THEN** error output includes the received byte count and states that retry will start from a clean temporary file/directory

#### Scenario: Cache validation failure is reported
- **WHEN** manifest or file validation fails
- **THEN** error output includes the cache path, failing file or manifest field, and recommended recovery command

#### Scenario: Cleanup failure is reported
- **WHEN** temporary artifact cleanup fails
- **THEN** output includes the path and OS error
- **AND** system does not claim cleanup success for that path

### Requirement: Download observability uses existing interfaces
Progress and errors SHALL use existing dependency-free channels.

#### Scenario: Progress uses ProgressReporter
- **WHEN** downloading a model in any CLI path
- **THEN** implementation reports progress through `ProgressReporter::planned()`, `cache_hit()`, `file_started()`, `file_progress()`, and `file_finished()`

#### Scenario: Errors flow through standard results
- **WHEN** a download error occurs
- **THEN** error is returned through the existing `Result`/error type path and is rendered by the CLI

#### Scenario: No logging dependency is required
- **WHEN** this change is implemented
- **THEN** it does not require adding `tracing`, `log`, `env_logger`, or a `RUST_LOG` configuration path

### Requirement: Cache status command provides observability
Users SHALL be able to inspect cache health and debug model cache issues without
knowing the on-disk layout.

#### Scenario: Cache status shows validation results
- **WHEN** user runs `quaid model status`
- **THEN** output includes status, files, size, manifest state, and last modified time for each recognized cache entry

#### Scenario: Cache status identifies problems
- **WHEN** a cache has issues
- **THEN** status output labels it as incomplete, corrupted, stale temporary, active temporary, or complete
