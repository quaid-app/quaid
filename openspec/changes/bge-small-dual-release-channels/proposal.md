## Why

GigaBrain currently has a mismatch between its product promise and its shipped binary story: the docs still describe an offline embedded-weight build, while the actual release lane only ships the slim cached/downloaded path. We need to restore an honest, reviewable release surface that supports both airgapped users and smaller-networked installs without widening scope into full multi-model support.

## What Changes

- Add a dual-release distribution strategy for **BGE-small-en-v1.5 only** with two supported channels:
  - **airgapped**: embedded model weights, zero network required at runtime
  - **online**: slim binary that downloads or uses cached BGE-small weights
- Update build and release workflows to produce, name, verify, and publish both channel variants for each supported platform.
- Update install surfaces (GitHub Releases, shell installer, staged npm wrapper, docs) so users can intentionally choose the airgapped or online channel.
- Make the project and user documentation truthful and synchronized around the dual-channel release story for `v0.9.1`.
- Explicitly defer base/large model support and any global runtime `--model small|base|large` UX.

## Capabilities

### New Capabilities
- `dual-release-distribution`: Produce and publish two BGE-small release channels (`airgapped` and `online`) with clear artifact naming, install selection, and verification.
- `airgapped-install-selection`: Allow installers and release guidance to select between the airgapped embedded artifact and the slim online artifact without changing application behavior.

### Modified Capabilities
- `documentation-accuracy`: Public and project docs must describe the real dual-channel release surface, the supported install paths, and the explicitly deferred model-family work.
- `release-readiness`: Release workflows and validation must cover both supported BGE-small channels instead of a single binary lane.

## Impact

- `Cargo.toml`, build features, and model-loading code in `src/core/inference.rs`
- Release/build/install surfaces in `.github/workflows/**`, `scripts/install.sh`, and `packages/gbrain-npm/**`
- User-facing and project-facing docs in `README.md`, `CLAUDE.md`, `docs/**`, and `website/**`
- Release assets, checksum generation, installer variant selection, and validation coverage for `v0.9.1`
