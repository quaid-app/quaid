## MODIFIED Requirements

### Requirement: Download errors do not leave orphaned temporary artifacts
The system SHALL clean up temporary download artifacts created by Quaid on all
download failure paths controlled by Quaid.

**Previous behavior**: Temporary directories/files could persist if downloads
failed on some paths, especially around early returns, hash failures, or
per-file embedding downloads.

**Updated behavior**: Extraction and embedding downloads use scope-bound cleanup
guards so temporary paths are removed on ordinary Rust error propagation. If the
operating system refuses cleanup, the failure is reported and the path remains
eligible for `quaid model clean`.

#### Scenario: Guard cleanup on any controlled error
- **WHEN** a download fails for any reason after a temporary path is created
- **THEN** the cleanup guard attempts to remove the temporary path before the error reaches the caller

#### Scenario: Cleanup failure is explicit
- **WHEN** cleanup cannot remove temporary files because the OS reports permission denied or in use
- **THEN** system reports the cleanup failure and does not claim the path was removed

#### Scenario: Rename race leaves valid cache
- **WHEN** temporary rename to the final cache location fails because another process produced a valid final cache first
- **THEN** system verifies the final cache, removes the local temporary artifact, and returns a cache hit

### Requirement: Model caches can be validated without unnecessary re-download
The system SHALL automatically accept, repair, or reject existing caches based on
trusted validation evidence.

**Previous behavior**: Missing extraction manifests caused otherwise complete
caches to be treated as invalid and re-downloaded. Embedding caches had no
manifest/status surface.

**Updated behavior**: The system can generate or upgrade manifests when expected
files and digests are known, accepts legacy manifests, and fails closed when a
cache cannot be trusted.

#### Scenario: Lazy manifest generation for source-verifiable cache
- **WHEN** model cache exists but `manifest.json` is missing and all expected files match trusted source pins
- **THEN** system creates a versioned manifest without re-downloading the model

#### Scenario: Legacy manifest accepted
- **WHEN** model cache has a versionless manifest and all referenced files verify
- **THEN** system accepts the cache and may upgrade the manifest to version 1

#### Scenario: Untrusted manifest generation is refused
- **WHEN** a custom/unpinned cache lacks a manifest and trusted expected digests are unavailable
- **THEN** system refuses to generate a manifest from arbitrary on-disk files and reports clear recovery instructions

#### Scenario: Validation failure prevents silent cache acceptance
- **WHEN** cache files do not match expected hashes
- **THEN** system marks the cache corrupted and refuses to load it

### Requirement: Stale temporary artifacts are actively cleaned
The system SHALL remove old incomplete temporary download artifacts to prevent
disk accumulation.

**Previous behavior**: Extraction stale directories were only scavenged before
the next extraction model download, and embedding temp files had no shared
cleanup path.

**Updated behavior**: Pre-download scavenging remains, and users can manually
trigger cache cleanup through `quaid model clean`.

#### Scenario: Manual model cache cleanup
- **WHEN** user runs `quaid model clean --all --force`
- **THEN** system removes stale temporary artifacts and incomplete/corrupted caches
- **AND** keeps complete verified caches

#### Scenario: Pre-download scavenging still occurs
- **WHEN** model download is initiated
- **THEN** system scavenges stale temporary artifacts before starting download

#### Scenario: TTL is configurable
- **WHEN** `QUAID_STALE_MODEL_CACHE_TTL_SECS` is set
- **THEN** system uses that value to determine stale temporary artifact age

### Requirement: Download state transitions are observable
The system SHALL provide clear user-facing output for download lifecycle events.

**Previous behavior**: Download progress was reported, but errors and cleanup
outcomes were not consistently actionable.

**Updated behavior**: Significant events are surfaced through the existing
progress reporter and command errors without adding a logging dependency.

#### Scenario: Download start is reported
- **WHEN** model download begins
- **THEN** system reports the alias, source repo, revision, and planned file count

#### Scenario: Download error includes context
- **WHEN** download fails
- **THEN** system reports the file, path or URL when relevant, cause, cleanup outcome, and recovery command

#### Scenario: Successful completion is reported
- **WHEN** download completes and cache is verified
- **THEN** system reports the cache path, file count, total size when known, and elapsed time
