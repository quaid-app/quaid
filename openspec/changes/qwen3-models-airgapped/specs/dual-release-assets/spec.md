## MODIFIED Requirements

### Requirement: Release workflow publishes both BGE-small channels
The release workflow SHALL publish a single Quaid binary per supported platform. The embedded-vs-online channel split is removed: every binary resolves its models from the local cache, downloading them once on first use. Supported platforms SHALL remain `darwin-arm64`, `darwin-x86_64`, `linux-x86_64`, and `linux-aarch64`.

#### Scenario: Tag build emits one binary per platform
- **WHEN** a semver tag build runs
- **THEN** the workflow produces exactly one `quaid` binary (plus checksum) for each of `darwin-arm64`, `darwin-x86_64`, `linux-x86_64`, and `linux-aarch64`, with no `airgapped`/`online` variants

### Requirement: Release assets use a stable channel suffix contract
Each release asset SHALL be named `quaid-<platform>` and each checksum `quaid-<platform>.sha256`. There SHALL be no `-airgapped` / `-online` channel suffix.

#### Scenario: Artifact manifest is verified after download
- **WHEN** the release job downloads build artifacts before publishing
- **THEN** it fails if any expected `quaid-<platform>` or matching `.sha256` file is missing

### Requirement: Online assets preserve the existing slim-download behavior
The single release binary SHALL exclude embedded model weights and SHALL provision its configured embedding and extraction models from the download/cache path on first use, consistent with the `airgapped-model-provisioning` capability.

#### Scenario: Binary provisions models on first use
- **WHEN** a freshly installed binary first needs embeddings or extraction with no cached model
- **THEN** it downloads and caches the configured model rather than reading embedded bytes

## REMOVED Requirements

### Requirement: Airgapped assets embed the BGE-small model bundle
**Reason**: "Airgapped" is redefined as local-only inference (no cloud, no egress), decoupled from embedded packaging. The new default models (~1.2 GB embedder, ~2.5 GB extractor) cannot be compiled into the binary, and the single channel provisions models on first use even under the airgapped posture.
**Migration**: Airgapped installs fetch and hook up their default models automatically on first use (see `airgapped-model-provisioning`); after the one-time fetch they operate fully offline.

### Requirement: Both channels keep one embedding schema contract
**Reason**: There is now one channel, not two, and the default embedding dimension changes from 384 to 1024 — a deliberate pre-release breaking change that requires a full re-embed.
**Migration**: Pre-change databases are not interoperable; re-initialize and re-embed (`quaid init` + `quaid embed --all`). No in-place schema migration is provided.
