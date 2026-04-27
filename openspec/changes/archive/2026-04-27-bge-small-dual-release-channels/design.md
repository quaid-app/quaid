## Context

Quaid already has two partial ingredients for dual distribution, but no coherent contract:

- `Cargo.toml` names `embed-model` and `online-model` features.
- `src/core/inference.rs` already has an online download path and a cache-only fallback path.
- `release.yml`, `scripts/install.sh`, and `packages/quaid-npm/scripts/postinstall.js` still assume one asset per platform.

The active team decision is narrower than the earlier distribution discussion: `v0.9.1` ships only BGE-small in two release channels. That keeps the embedding dimension fixed at 384, preserves DB compatibility, and avoids inventing runtime `--model` selection or base/large support before the project has a migration story for multi-model brains.

## Goals / Non-Goals

**Goals:**
- Define a build-time-only dual-channel release contract for `v0.9.1`.
- Make GitHub Release assets, installers, npm postinstall, and docs agree on the two channel names and defaults.
- Keep the embedding/search API and database contract identical across both channels.
- Add validation gates that make release drift visible before tagging.

**Non-Goals:**
- Add BGE-base or BGE-large support.
- Add `--model small|base|large`, config-table model selection, or any other runtime model-family UX.
- Change embedding dimensions, vec table layout, or require re-embedding existing brains.
- Add new package registries or Windows-specific installer work.

## Decisions

### 1. Treat release channels as build-time packaging only

**Decision:** `v0.9.1` defines exactly two BGE-small channels:

- **airgapped** — compiled with embedded BGE-small assets and expected to initialize embeddings without network or a pre-existing Hugging Face cache
- **online** — compiled without embedded weights and expected to download/cache BGE-small when embedding is first needed

No runtime flag chooses between them. Users pick a release asset or install path; the binary behavior is fixed by how it was built.

**Rationale:** The team-approved narrower scope is about distribution, not multi-model product UX. Keeping selection at build/install time preserves a single 384-dimension schema and avoids re-embedding/migration work.

**Alternative considered:** Add a runtime `--model` flag or ship small/base/large variants. Rejected because it multiplies release/test/docs burden and needs DB lifecycle semantics the team explicitly deferred.

### 2. Use explicit channel suffixes in release assets

**Decision:** Release assets SHALL be named `quaid-<platform>-airgapped` and `quaid-<platform>-online`, with matching `.sha256` sidecars.

Examples:
- `quaid-darwin-arm64-airgapped`
- `quaid-darwin-arm64-airgapped.sha256`
- `quaid-linux-x86_64-online`
- `quaid-linux-x86_64-online.sha256`

**Rationale:** Suffixing the existing platform-oriented name keeps the current install contract recognizable while making the channel visible everywhere: release UI, scripts, checksums, and docs.

**Alternative considered:** Prefix the channel (`quaid-airgapped-<platform>`) or use separate tag/release names. Rejected because it makes platform matching harder in the existing installer/npm code and creates unnecessary release fragmentation.

### 3. Installer defaults follow audience, not symmetry

**Decision:**
- `scripts/install.sh` defaults to `airgapped` and accepts `QUAID_CHANNEL=airgapped|online`.
- `packages/quaid-npm/scripts/postinstall.js` downloads the `online` asset only and points users to GitHub Releases if they need the offline-safe build.
- README/docs/release notes document both channels and state those defaults explicitly.

**Rationale:** The curl installer is the easiest path for shell users who often want a self-contained binary; npm users already have networked Node environments and benefit most from the slimmer package/download path. This keeps one npm package and one shell installer instead of multiplying wrappers.

**Alternative considered:** Make every installer surface prompt for a channel or publish two npm packages. Rejected because it adds UX and packaging complexity without adding product capability.

### 4. The inference contract stays 384-dim and channel-agnostic

**Decision:** `src/core/inference.rs` continues exposing one embedding/search API. Channel differences are limited to how model files are sourced:
- embedded bytes/resources for `airgapped`
- Hugging Face download/cache path for `online`

The reported model family remains BGE-small-en-v1.5 and the DB/vector metadata remains 384-dimensional in both channels.

**Rationale:** Search/index compatibility is the reason the narrow scope was approved. Existing brains and release assets must stay interchangeable at the database layer.

**Alternative considered:** Distinguish channels in DB metadata or treat them as separate model identities. Rejected because the semantic model is the same; only packaging differs.

### 5. Build-time model sourcing belongs in the release pipeline, not in git history

**Decision:** The implementation should obtain the BGE-small asset bundle during build/release execution for the `airgapped` channel and embed it into the binary through build-time plumbing. The repo should not require committing large safetensor blobs just to produce `v0.9.1`.

**Rationale:** The approved scope is dual release channels, not repository bloat. CI can stage the exact bundle needed for the embedded build while keeping source control lean and the online path unchanged.

**Alternative considered:** Commit the model bundle to the repository. Rejected because it bloats clones and complicates normal contributor workflows.

### 6. Dual-channel release is gated by explicit build and install validation

**Decision:** `v0.9.1` does not ship until all of the following are true:
- both channel feature sets build for all supported platforms in `release.yml`
- expected binary/checksum asset names are verified after artifact download
- shell installer resolves both default (`airgapped`) and override (`online`) asset names correctly
- npm packaging validates against the `online` asset contract without bundling a binary
- docs/release notes explicitly state BGE-small-only, no base/large, and no runtime `--model` UX

**Rationale:** The risk here is operational drift, not algorithmic novelty. The gates should focus on naming, distribution behavior, and truthfulness.

**Alternative considered:** Rely on existing single-channel smoke tests. Rejected because they would allow silent regressions in one of the two new release paths.

## Risks / Trade-offs

- **[Airgapped embedding source is not yet wired]** → Mitigation: make model-bundle acquisition and embedding plumbing the first implementation slice before any doc/release claim changes land.
- **[Eight binaries plus checksums increase release complexity]** → Mitigation: keep the scope to one model family, verify a fixed manifest in CI, and standardize naming before docs are updated.
- **[Installer defaults could confuse users]** → Mitigation: document the defaults in README, release notes, and installer help text, and give `scripts/install.sh` a single env-var override instead of hidden behavior.
- **[npm users may expect offline behavior]** → Mitigation: document that npm installs the slim/online channel and point offline users to the `airgapped` GitHub Release asset or shell installer.
- **[Doc/spec drift may reintroduce unapproved model UX promises]** → Mitigation: add validation/reviewer checklist items that grep for base/large or `--model` language before tagging.

## Migration Plan

1. Land the OpenSpec artifacts so Fry has a locked scope for `v0.9.1`.
2. Implement build-time model sourcing plus feature wiring in `Cargo.toml` and `src/core/inference.rs`.
3. Expand `release.yml` to emit both channels and verify the full asset manifest.
4. Update `scripts/install.sh`, npm postinstall, and all install/release docs against the new asset names and defaults.
5. Run the dual-channel validation gates and collect Bender/Kif/Leela sign-off before tagging `v0.9.1`.

Rollback is straightforward: revert the dual-channel workflow/installer/docs changes and delete the `v0.9.1` release assets before re-tagging. No schema migration or memory-data rewrite is involved.

## Open Questions

- None at proposal time. Any newly discovered blocker must be captured as a design/task delta before implementation continues.
