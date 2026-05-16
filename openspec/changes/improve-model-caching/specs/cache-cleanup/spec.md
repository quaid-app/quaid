## ADDED Requirements

### Requirement: User can inspect model cache cleanup candidates
Users SHALL be able to preview stale, incomplete, or corrupted model cache
artifacts without removing them.

#### Scenario: List cleanup candidates
- **WHEN** user runs `quaid model clean --list`
- **THEN** system displays cleanup candidates from the model cache root, including extraction temp directories, embedding temp files, incomplete caches, and corrupted caches
- **AND** output includes path, cache family, reason, age, and size
- **AND** no files or directories are removed

#### Scenario: List specific model cache candidates
- **WHEN** user runs `quaid model clean --list phi-3.5-mini`
- **THEN** system displays cleanup candidates associated with `phi-3.5-mini`
- **AND** complete verified caches associated with the alias are shown as kept unless explicitly targeted for removal

#### Scenario: Empty list when no cleanup candidates exist
- **WHEN** user runs `quaid model clean --list` but no stale, incomplete, or corrupted artifacts exist
- **THEN** system prints "No model cache cleanup candidates found" and exits successfully

### Requirement: User can remove stale and broken model cache artifacts
Users SHALL be able to remove incomplete, corrupted, or stale temporary model
cache artifacts to free disk space and recover from download failures.

#### Scenario: Remove with confirmation
- **WHEN** user runs `quaid model clean --all`
- **THEN** system prompts for confirmation with a summary of what will be removed
- **AND** if user confirms, removes all eligible stale/incomplete/corrupted artifacts
- **AND** complete verified caches are not removed

#### Scenario: Force removal without confirmation
- **WHEN** user runs `quaid model clean --all --force`
- **THEN** system immediately removes eligible stale/incomplete/corrupted artifacts without prompting
- **AND** complete verified caches are still not removed

#### Scenario: Remove specific model cache
- **WHEN** user runs `quaid model clean phi-3.5-mini --force`
- **THEN** system removes cache artifacts associated with `phi-3.5-mini`
- **AND** because the alias is explicit and `--force` is present, the complete verified cache for that alias MAY be removed

#### Scenario: Partial failures do not stop cleanup
- **WHEN** system is removing multiple cleanup candidates and one fails to delete
- **THEN** system continues removing other requested candidates
- **AND** reports failed paths with error reasons at the end
- **AND** exits non-zero

### Requirement: Cleanup safety guards prevent accidental data loss
The system SHALL implement safety mechanisms that prevent broad cleanup from
removing complete verified model caches.

#### Scenario: Complete caches excluded from broad cleanup
- **WHEN** user runs `quaid model clean --all --force`
- **THEN** system removes stale/incomplete/corrupted artifacts only
- **AND** keeps complete verified caches

#### Scenario: Active downloads are skipped
- **WHEN** cleanup sees a temporary download artifact with a recent `.downloading` heartbeat or recent modified time
- **THEN** system marks it as active and skips removal

#### Scenario: Expired temporary artifacts are eligible
- **WHEN** cleanup sees a temporary download artifact older than the configured stale TTL and without a recent heartbeat
- **THEN** system marks it as stale and eligible for removal

#### Scenario: User shown what is not being removed
- **WHEN** user runs `quaid model clean --list`
- **THEN** output clearly distinguishes between "would remove" and "kept"

### Requirement: Cache cleanup respects configured cache root
The system SHALL only inspect and remove paths inside the resolved model cache
root.

#### Scenario: Custom model cache root
- **WHEN** `QUAID_MODEL_CACHE_DIR` is set and user runs `quaid model clean --list`
- **THEN** system inspects only that directory tree
- **AND** does not inspect `~/.quaid/models`

#### Scenario: Path traversal is rejected
- **WHEN** a manifest or cache entry contains an absolute path or parent-directory traversal
- **THEN** system reports the entry as invalid
- **AND** cleanup does not follow that path outside the cache root
