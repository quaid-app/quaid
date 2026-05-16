## ADDED Requirements

### Requirement: System validates existing model caches
The system SHALL automatically validate cached models and detect incomplete or corrupted caches.

#### Scenario: Manifest found and valid
- **WHEN** a cached model is accessed
- **THEN** system checks manifest.json exists and verifies all referenced files are present with correct SHA-256 hashes

#### Scenario: Manifest missing but files complete
- **WHEN** a cached model exists without manifest.json but all expected files are present with correct hashes
- **THEN** system generates manifest.json automatically without requiring re-download

#### Scenario: Manifest missing and files incomplete
- **WHEN** a cached model exists without manifest.json and is missing some expected files
- **THEN** system marks cache as incomplete and recommends re-download via `quaid model pull`

#### Scenario: Manifest invalid - hash mismatch
- **WHEN** manifest.json exists but file hashes don't match actual files
- **THEN** system reports validation failure and recommends cache cleanup + re-download

#### Scenario: Manifest invalid - missing files
- **WHEN** manifest.json exists but references files that don't exist on disk
- **THEN** system reports cache corruption and recommends removal via `quaid cache clean`

### Requirement: System reports cache health
Users SHALL be able to query cache status and validation results for debugging and monitoring.

#### Scenario: Check cache status
- **WHEN** user runs `quaid cache status`
- **THEN** system lists all cached models with status (complete/incomplete/corrupted), manifest validity, and last modified time

#### Scenario: Check specific model cache
- **WHEN** user runs `quaid cache status phi-3.5-mini`
- **THEN** system displays cache directory path, manifest status, file count, total size, and validation results

#### Scenario: Verbose cache inspection
- **WHEN** user runs `quaid cache status --verbose`
- **THEN** system displays individual file paths, sizes, and SHA-256 hashes from manifest

### Requirement: Cache validation is non-blocking
Cache validation failures SHALL not prevent model usage if alternative paths exist, but SHALL be logged and reported.

#### Scenario: Validation failure doesn't block load
- **WHEN** cache validation fails for a model
- **THEN** system logs the failure and either uses a fallback cache or errors with clear recovery instructions

#### Scenario: Lazy manifest generation succeeds silently
- **WHEN** manifest is auto-generated for a previously-cached model
- **THEN** system logs at trace level but doesn't surface as a warning or error

### Requirement: Manifest format is stable and versioned
Manifest files SHALL use a consistent format that survives model lifecycle upgrades.

#### Scenario: Manifest includes version
- **WHEN** manifest.json is created
- **THEN** it includes a `manifest_version` field for future format changes

#### Scenario: Manifest includes all required metadata
- **WHEN** manifest.json is read
- **THEN** it contains: requested_alias, repo_id, revision, file list with paths and SHA-256 hashes, timestamp
