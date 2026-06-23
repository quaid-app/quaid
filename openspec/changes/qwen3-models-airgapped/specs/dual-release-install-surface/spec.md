## MODIFIED Requirements

### Requirement: Shell installer defaults to the airgapped channel
`scripts/install.sh` SHALL resolve the single `quaid-<platform>` asset for the detected platform. The `QUAID_CHANNEL` channel selector is removed (or accepted and ignored with a deprecation notice); there is no embedded-vs-online choice because every binary provisions models on first use.

#### Scenario: Default shell install resolves the single asset
- **WHEN** a user runs `curl .../install.sh | sh`
- **THEN** the installer downloads `quaid-<platform>` and its checksum for the resolved platform, with no channel branching

#### Scenario: Deprecated channel selector does not break install
- **WHEN** a user sets `QUAID_CHANNEL` before invoking `install.sh`
- **THEN** the installer still resolves the single `quaid-<platform>` asset and (optionally) prints a deprecation notice

### Requirement: npm postinstall uses the online channel only
The npm package SHALL download the single `quaid-<platform>` release asset for the current supported platform and SHALL NOT bundle the binary in the npm tarball.

#### Scenario: npm install resolves the single asset
- **WHEN** `packages/quaid-npm/scripts/postinstall.js` runs on a supported platform
- **THEN** it downloads `quaid-<platform>`, verifies its checksum, and installs it as the package binary

#### Scenario: npm package stays slim
- **WHEN** `npm pack --dry-run` is executed for the `quaid` package
- **THEN** the packed files exclude downloaded binaries and only include the wrapper/package metadata needed to fetch the asset at install time

### Requirement: Install docs explain both channels and their defaults
README, docs, website content, and release notes SHALL explain that there is a single binary per platform that runs all inference locally (the airgapped privacy guarantee) and provisions its models on first use. They SHALL describe the default models (`Qwen3-Embedding-0.6B`, `Qwen3-4B-Instruct-2507`) and SHALL NOT describe an embedded-vs-online channel choice.

#### Scenario: Public docs describe the single-binary provisioning model
- **WHEN** a user reads install guidance
- **THEN** they can tell that one binary is shipped per platform, that it downloads its models once on first use, and that all inference stays local afterward

### Requirement: Public install copy does not promise unapproved model UX
Install and release copy SHALL accurately describe the supported defaults (`Qwen3-Embedding-0.6B` for embeddings, `Qwen3-4B-Instruct-2507` for extraction) and the supported runtime model-selection surface (`QUAID_MODEL` / `--model` for embeddings; the extraction model alias). Copy SHALL NOT promise model families or selectors that are not actually shipped.

#### Scenario: Install copy matches shipped model UX
- **WHEN** release notes and install docs are reviewed
- **THEN** they describe the Qwen3 defaults and the actual model-selection surface, with no claims for unshipped models or selectors
