## MODIFIED Requirements

### Requirement: Dual-channel builds are part of the release gate
Before a release is tagged, the repository SHALL validate the single build path (one binary per platform, models provisioned on first use). The removed embedded-vs-online feature split SHALL NOT be exercised. Validation SHALL include the model-provisioning path (first-use download + integrity verification + hook-up) and the airgapped (offline-after-fetch) behavior.

#### Scenario: Single build path is exercised before release
- **WHEN** release validation commands run for this change
- **THEN** the single feature set compiles for all supported targets and passes the agreed test suite, including a model-provisioning smoke check

### Requirement: Installer and package smoke tests cover channel-specific behavior
The validation plan SHALL include smoke checks for the shell installer resolving the single `quaid-<platform>` asset and for npm postinstall downloading that asset without bundling a binary.

#### Scenario: Installer and npm paths match the single-asset names
- **WHEN** release validation is executed
- **THEN** the shell installer proves it resolves `quaid-<platform>`, and npm packaging proves the single-asset mapping plus a no-binary tarball

### Requirement: Release review includes documentation truthfulness checks
The final reviewer gate SHALL verify that release notes and docs are synchronized with the single-binary, provision-on-first-use, airgapped-privacy contract and the Qwen3 default models, and do not reintroduce embedded-vs-online channel claims or unshipped model promises.

#### Scenario: Review blocks stale channel or model claims
- **WHEN** the release surface is reviewed
- **THEN** the release is blocked until any mention of embedded-vs-online channels, BGE-small-as-default, or unshipped model selectors is corrected
