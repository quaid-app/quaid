# Fry — Issue #81 release-ready decision

- **Date:** 2026-04-25
- **Decision:** Ship PR #84 as patch release `v0.9.8`, and rewrite the GitHub release body to describe the empty-root watcher hotfix instead of reusing the prior `v0.9.7` macOS release-contract notes.
- **Why:** The code fix in PR #84 is already the next patch candidate, so leaving product/version surfaces at `0.9.7` would mislabel the release and stale release-note prose would describe the wrong user-visible repair.
- **Scope touched:** `Cargo.toml`, `Cargo.lock`, `packages/gbrain-npm/package.json`, `src/core/inference.rs`, `README.md`, `docs/getting-started.md`, `website/src/content/docs/guides/install.mdx`, `.github/workflows/release.yml`.
