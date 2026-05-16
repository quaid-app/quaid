## ADDED Requirements

### Requirement: User can list stale model caches
Users SHALL be able to preview which stale or incomplete model caches would be removed without actually removing them.

#### Scenario: List stale directories
- **WHEN** user runs `quaid cache clean --list`
- **THEN** system displays all temporary download directories (age-based staleness check) and incomplete caches (missing manifest/failed validation), with their paths and sizes

#### Scenario: List specific model cache
- **WHEN** user runs `quaid cache clean --list phi-3.5-mini`
- **THEN** system displays all caches related to phi-3.5-mini (both temp and complete directories)

#### Scenario: Empty list when no stale caches
- **WHEN** user runs `quaid cache clean --list` but no stale caches exist
- **THEN** system prints "No stale caches found" and exits successfully

### Requirement: User can remove stale model caches
Users SHALL be able to remove incomplete, broken, or temporary model caches to free up disk space and recover from download failures.

#### Scenario: Remove with confirmation
- **WHEN** user runs `quaid cache clean --all`
- **THEN** system prompts for confirmation with a summary of what will be removed
- **AND** if user confirms, removes all identified stale directories

#### Scenario: Force removal without confirmation
- **WHEN** user runs `quaid cache clean --all --force`
- **THEN** system immediately removes all stale caches without prompting

#### Scenario: Remove specific model cache
- **WHEN** user runs `quaid cache clean phi-3.5-mini --force`
- **THEN** system removes the cache directory for phi-3.5-mini only

#### Scenario: Partial failures don't stop cleanup
- **WHEN** system is removing multiple stale directories and one fails to delete
- **THEN** system continues removing others and reports which directories failed at the end

### Requirement: User receives clear cache cleanup reporting
Users SHALL receive clear feedback about what was removed, what failed, and cache health statistics after cleanup.

#### Scenario: Success report shows freed space
- **WHEN** cache cleanup completes successfully
- **THEN** system reports: number of directories removed, total disk space freed, paths of removed directories

#### Scenario: Partial failure report identifies problems
- **WHEN** some directories fail to remove
- **THEN** system reports successful removals and lists failed directories with error reasons (permission denied, in use, etc.)

### Requirement: Safety guards prevent accidental data loss
The system SHALL implement safety mechanisms to prevent users from accidentally removing non-stale caches.

#### Scenario: Non-stale caches excluded by default
- **WHEN** user runs `quaid cache clean --all` without `--force`
- **THEN** system only removes stale temp directories and incomplete caches, not recent/complete caches

#### Scenario: User shown what is not being removed
- **WHEN** user runs `quaid cache clean --list`
- **THEN** output clearly distinguishes between "stale (will be removed)" and "active/complete (will be kept)"
