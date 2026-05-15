# release-housekeeping Specification

## ADDED Requirements

### Requirement: Patch release housekeeping gate

Before tagging a patch release for a bug fix already landed on `main`, the release lane SHALL reconcile public roadmap truth, issue-tracker truth, and remote-branch cleanup status from the same main-branch reality.

#### Scenario: unreleased fix exists on main

- **GIVEN** the latest published tag is behind `origin/main`
- **AND** public roadmap or release-facing docs still describe stale shipped state
- **WHEN** the next patch release lane is prepared
- **THEN** the release lane MUST start from `origin/main`
- **AND** the public roadmap/release truth pass MUST be reviewed before the tag is pushed
- **AND** the issue shortlist and branch-pruning plan MUST be derived from the same `origin/main` snapshot

### Requirement: Remote branch pruning safety

Remote branch cleanup SHALL only auto-delete branches whose exact remote tip SHA is already reachable from `origin/main`.

#### Scenario: stale-looking branch still has unique commits

- **GIVEN** a remote branch name appears stale
- **BUT** its tip SHA is not an ancestor of `origin/main`
- **WHEN** housekeeping runs branch cleanup
- **THEN** the branch MUST NOT be auto-deleted
- **AND** the batch MUST route that branch for owner review instead

#### Scenario: merged branch tip is fully contained in main

- **GIVEN** a remote branch tip SHA is an ancestor of `origin/main`
- **WHEN** housekeeping runs branch cleanup
- **THEN** that branch MAY be deleted as a merged branch candidate
