updated_at: 2026-04-17T14:45:04Z
focus_area: v0.9.2 release surface bump
active_issues: [version-surface-bump]
active_branch: release/v0.9.2
---

# What We're Focused On

**Active change:** `v0.9.2-release-bump` — update version surfaces and docs to `v0.9.2`.

**Scope:**
- Update package versions (Cargo.toml, npm package) and user-agent strings.
- Refresh README/docs/website copy and release workflow text to match `v0.9.2`.

**Validation:**
- Local: `cargo fmt --all --check`.
- Packaging: `npm pack --dry-run` from `packages/gbrain-npm/` (when available).
- Full build/test remains CI-only in this Windows environment.
