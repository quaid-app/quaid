## ADDED Requirements

### Requirement: Dual-channel builds are part of the release gate
Before `v0.9.1` is tagged, the repository SHALL validate the codebase against both the default/airgapped build path and the online build path.

#### Scenario: Both channel feature sets are exercised before release
- **WHEN** Fry runs the release validation commands for this change
- **THEN** both the `airgapped` and `online` feature combinations compile and pass the agreed test suite

### Requirement: Installer and package smoke tests cover channel-specific behavior
The validation plan SHALL include smoke checks for the shell installer's default and override channel resolution and for npm postinstall downloading the `online` asset without bundling a binary.

#### Scenario: Installer and npm paths match the new asset names
- **WHEN** release validation is executed
- **THEN** the shell installer proves `airgapped` by default and `online` when overridden, and npm packaging proves the `online` asset mapping plus no-binary tarball contents

### Requirement: Release review includes documentation truthfulness checks
The final reviewer gate SHALL verify that release notes and docs are synchronized with the dual-channel asset contract and do not reintroduce unapproved model-family claims.

#### Scenario: Review blocks unsupported public promises
- **WHEN** Leela, Bender, or Kif review the `v0.9.1` release surface
- **THEN** the release is blocked until any mention of base/large support or runtime `--model` selection is removed

### Requirement: `v0.9.1` ships only after explicit reviewer sign-off
`v0.9.1` SHALL require explicit sign-off from the release lead and testers after the dual-channel build, installer, npm, and doc checks complete.

#### Scenario: Tagging waits for reviewer confirmation
- **WHEN** all implementation tasks are marked complete
- **THEN** the `v0.9.1` tag is not pushed until the required reviewer sign-offs are recorded against the completed validation checklist
