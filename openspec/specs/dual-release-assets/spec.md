# dual-release-assets Specification

## Purpose
TBD - created by archiving change bge-small-dual-release-channels. Update Purpose after archive.
## Requirements
### Requirement: Release workflow publishes both BGE-small channels
The release workflow SHALL publish two BGE-small binaries for every supported release platform in `v0.9.1`: `airgapped` and `online`.

#### Scenario: Tag build emits both channels for each platform
- **WHEN** a semver tag build runs for `v0.9.1`
- **THEN** the workflow produces `airgapped` and `online` binaries for `darwin-arm64`, `darwin-x86_64`, `linux-x86_64`, and `linux-aarch64`

### Requirement: Release assets use a stable channel suffix contract
Each dual-channel release asset SHALL be named `quaid-<platform>-<channel>` and each checksum SHALL be named `quaid-<platform>-<channel>.sha256`, where `<channel>` is `airgapped` or `online`.

#### Scenario: Artifact manifest is verified after download
- **WHEN** the release job downloads build artifacts before publishing
- **THEN** it fails if any expected `quaid-<platform>-airgapped`, `quaid-<platform>-online`, or matching `.sha256` file is missing

### Requirement: Airgapped assets embed the BGE-small model bundle
The `airgapped` channel SHALL include the BGE-small-en-v1.5 model bundle inside the binary so embedding initialization does not require a network call or a pre-existing Hugging Face cache.

#### Scenario: Airgapped build can initialize offline
- **WHEN** an `airgapped` binary runs in an environment with no network and no existing model cache
- **THEN** embedding initialization succeeds without attempting a model download

### Requirement: Online assets preserve the existing slim-download behavior
The `online` channel SHALL exclude embedded BGE-small weights and SHALL load BGE-small-en-v1.5 from the existing download/cache path when embeddings are first needed.

#### Scenario: Online build uses download-or-cache path
- **WHEN** an `online` binary first needs embeddings
- **THEN** it uses the Hugging Face download/cache path instead of embedded bytes while preserving the same BGE-small model family

### Requirement: Both channels keep one embedding schema contract
Both release channels SHALL continue using BGE-small-en-v1.5 with 384-dimensional embeddings so databases, vec rows, and existing pages remain interoperable across channels.

#### Scenario: Channel choice does not require re-embedding
- **WHEN** a user switches from an `online` binary to an `airgapped` binary or back
- **THEN** the existing memory database remains usable without schema migration or model-family conversion

