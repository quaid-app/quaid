## ADDED Requirements

### Requirement: GitHub Releases are the only supported binary distribution channel in this change
The project SHALL treat GitHub Releases and build-from-source as the only supported install paths covered by this change. npm global installation and simplified installer UX SHALL be documented as deferred follow-on work until they have their own approved implementation scope.

#### Scenario: User reads install guidance before npm packaging exists
- **WHEN** a public install page is updated under this change
- **THEN** it lists GitHub release binaries and build-from-source as supported now, and labels npm/installer work as planned later without presenting those commands as current

### Requirement: Release workflow publishes stable artifacts and checksums
The release workflow SHALL publish the supported platform binaries and matching `.sha256` files from CI using stable artifact names that match the public install documentation.

#### Scenario: Tag push creates a release
- **WHEN** a semver tag handled by the release workflow is pushed
- **THEN** the workflow builds each supported artifact, generates a matching checksum file, and attaches both files to the GitHub Release

#### Scenario: Required release artifact verification fails
- **WHEN** an expected artifact, checksum, or verification step fails during the release workflow
- **THEN** the workflow stops before publishing the GitHub Release

### Requirement: Release-facing review checklist exists before public ship
The release-ready surface SHALL include a reviewable checklist covering asset names, checksum wording, install guidance, and deferred-channel messaging before Zapp signs off on a public release.

#### Scenario: Zapp performs final release review
- **WHEN** the team prepares a public release from this change
- **THEN** Zapp can review a single checklist that names the release assets, public install text, and explicitly deferred npm/installer work
