## ADDED Requirements

### Requirement: System validates model caches before use
The system SHALL validate cached models and reject incomplete or corrupted
caches instead of silently using them.

#### Scenario: Manifest found and valid
- **WHEN** a cached extraction model is accessed and `manifest.json` exists
- **THEN** system validates the manifest structure, path safety, expected alias/repo/revision, and file presence
- **AND** system accepts the cache only if validation succeeds

#### Scenario: Fast validation detects changed file metadata
- **WHEN** a manifest records file size or modified time and an on-disk file no longer matches those values
- **THEN** system treats the cache as suspect
- **AND** performs full hash verification before accepting the cache

#### Scenario: Hash verification fails
- **WHEN** full verification finds a SHA-256 or source-pin mismatch
- **THEN** system reports cache corruption and recommends cleanup plus re-pull
- **AND** system does not use the corrupted cache

#### Scenario: Manifest references unsafe path
- **WHEN** `manifest.json` references an absolute path or parent-directory traversal
- **THEN** system rejects the manifest
- **AND** system does not open the referenced path

### Requirement: System safely generates or upgrades manifests
The system SHALL generate or upgrade manifest files only when it can prove the
cache's expected file set and digests from trusted model metadata.

#### Scenario: Legacy manifest remains valid
- **WHEN** a cached model has a versionless legacy manifest with valid existing fields and hashes
- **THEN** system accepts the cache
- **AND** MAY upgrade the manifest to version 1 after full verification

#### Scenario: Manifest missing but curated extraction files complete
- **WHEN** a curated extraction model cache exists without `manifest.json` and all source-pinned files are present with correct digests
- **THEN** system generates `manifest.json` automatically without requiring re-download

#### Scenario: Manifest missing but known embedding files complete
- **WHEN** a known embedding model cache exists without `manifest.json` and all pinned embedding files are present with correct digests
- **THEN** system generates or records equivalent manifest metadata without requiring re-download

#### Scenario: Manifest missing and files incomplete
- **WHEN** a cache exists without `manifest.json` and one or more expected files are missing
- **THEN** system marks cache as incomplete and recommends `quaid model clean <alias> --force` followed by `quaid model pull <alias>` when pull is supported

#### Scenario: Manifest missing for untrusted custom cache
- **WHEN** a custom or unpinned model cache exists without `manifest.json` and trusted expected digests are unavailable
- **THEN** system fails closed
- **AND** system does not generate a manifest by hashing the files that happen to be present

### Requirement: Manifest format is stable and backward compatible
Manifest files SHALL use a versioned format while preserving compatibility with
existing versionless manifests.

#### Scenario: New manifest includes version
- **WHEN** system creates a new manifest
- **THEN** it includes `manifest_version: 1`

#### Scenario: New manifest includes required metadata
- **WHEN** system creates a new manifest
- **THEN** it contains `requested_alias`, `repo_id`, `revision`, `created_at_unix`, and file entries with `path`, `sha256`, `size_bytes`, `modified_unix`, and `verified_from_source`

#### Scenario: Versionless manifest is treated as legacy
- **WHEN** system reads a manifest without `manifest_version`
- **THEN** it treats the manifest as legacy version 0
- **AND** validates all legacy fields before accepting it

### Requirement: User can query model cache health
Users SHALL be able to query cache status and validation results for debugging
and monitoring.

#### Scenario: Check all cache status
- **WHEN** user runs `quaid model status`
- **THEN** system lists recognized cached models and temporary artifacts with status, file count, size, manifest state, and last modified time

#### Scenario: Check specific model cache
- **WHEN** user runs `quaid model status phi-3.5-mini`
- **THEN** system displays cache directory path, manifest status, file count, total size, and validation result for matching cache entries

#### Scenario: Verbose cache inspection
- **WHEN** user runs `quaid model status --verbose`
- **THEN** system displays individual file paths, sizes, and manifest hashes where available

#### Scenario: Explicit full verification
- **WHEN** user runs `quaid model status --verify`
- **THEN** system performs full hash verification before reporting verified/corrupted status
- **AND** output indicates that full verification was performed
