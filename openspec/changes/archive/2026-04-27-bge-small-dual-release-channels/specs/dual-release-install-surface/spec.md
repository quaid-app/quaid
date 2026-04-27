## ADDED Requirements

### Requirement: Shell installer defaults to the airgapped channel
`scripts/install.sh` SHALL resolve `airgapped` assets by default and SHALL accept `QUAID_CHANNEL=airgapped|online` to override the selected release channel.

#### Scenario: Default shell install chooses the offline-safe asset
- **WHEN** a user runs `curl .../install.sh | sh` without setting `QUAID_CHANNEL`
- **THEN** the installer downloads `quaid-<platform>-airgapped` and its checksum for the resolved platform

#### Scenario: Shell installer override chooses the online asset
- **WHEN** a user sets `QUAID_CHANNEL=online` before invoking `install.sh`
- **THEN** the installer downloads `quaid-<platform>-online` and its checksum for the resolved platform

### Requirement: npm postinstall uses the online channel only
The npm package SHALL download the `online` release asset for the current supported platform and SHALL not bundle either release binary in the npm tarball.

#### Scenario: npm install resolves the online asset
- **WHEN** `packages/quaid-npm/scripts/postinstall.js` runs on a supported platform
- **THEN** it downloads `quaid-<platform>-online`, verifies its checksum, and installs it as the package binary

#### Scenario: npm package stays slim
- **WHEN** `npm pack --dry-run` is executed for the `quaid` package
- **THEN** the packed files exclude downloaded binaries and only include the wrapper/package metadata needed to fetch the `online` channel at install time

### Requirement: Install docs explain both channels and their defaults
README, docs, website content, and release notes SHALL explain the difference between `airgapped` and `online`, call out the default channel for each install surface, and provide manual asset examples for both channels.

#### Scenario: Public docs describe the supported release choice
- **WHEN** a user reads install guidance for `v0.9.1`
- **THEN** they can tell which channel the shell installer uses, which channel npm uses, and where to fetch the other channel manually

### Requirement: Public install copy does not promise unapproved model UX
Install and release copy SHALL state that `v0.9.1` supports BGE-small only and SHALL NOT claim base/large model support or a runtime `--model small|base|large` selector.

#### Scenario: Unsupported model-family UX stays out of public copy
- **WHEN** release notes and install docs are reviewed for `v0.9.1`
- **THEN** they describe only the two BGE-small release channels and omit any base/large or runtime model-selection promise
