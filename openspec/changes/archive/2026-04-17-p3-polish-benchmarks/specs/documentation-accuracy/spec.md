## ADDED Requirements

### Requirement: Public docs describe current repo reality honestly
`README.md` and public-facing documentation SHALL distinguish implemented behavior, in-progress work, and explicitly deferred work according to the current repository state and approved release scope.

#### Scenario: Feature is not yet ready for release
- **WHEN** a documented command, release channel, or workflow is still planned or deferred
- **THEN** the docs label it as planned/deferred instead of presenting it as currently available

#### Scenario: Stale status copy exists
- **WHEN** README or docs-site copy still describes an earlier project phase or outdated status
- **THEN** the copy is updated to the current release-readiness posture in this change

### Requirement: Supported-now and planned-later install paths are separated
Public install guidance SHALL separate supported install methods available now from future distribution ideas that are not yet implemented.

#### Scenario: Reader compares install options
- **WHEN** a user reads the install section in README or the docs site
- **THEN** supported-now paths are grouped separately from planned-later npm or installer work

### Requirement: README and docs site share one status/install message
The public project status, supported release channels, and deferred-channel wording SHALL match across `README.md` and the published docs site.

#### Scenario: User reads both README and docs site
- **WHEN** the same user checks project status and install guidance in both places
- **THEN** the two sources communicate the same release state and supported distribution paths
