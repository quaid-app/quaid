updated_at: 2026-04-19T00:00:00Z
focus_area: bge-small-dual-release-channels — v0.9.1 dual-release lane
active_issues: [D.1-validation-run, D.2-pr-open]
active_branch: release/v0.9.1-dual-release
---

# What We're Focused On

**Active change:** `bge-small-dual-release-channels` — `v0.9.1` dual-release BGE-small channels (`airgapped` + `online`).

OpenSpec change: `openspec/changes/bge-small-dual-release-channels/` — fully populated and machine-readable.

**Done (A–C):**
- **A.1** ✅ — OpenSpec artifacts complete and machine-parsable (corrected tasks.md)
- **A.2** ✅ — `embedded-model` Cargo feature + `build.rs` model-bundle acquisition wired
- **A.3** ✅ — `src/core/inference.rs` dual-channel feature gate with `compile_error!` guard
- **B.1** ✅ — `release.yml` builds both channels for all 4 platforms with `.sha256` sidecars and manifest verification
- **B.2** ✅ — `scripts/install.sh` defaults to `airgapped`, accepts `GBRAIN_CHANNEL=airgapped|online`
- **B.3** ✅ — `postinstall.js` defaults to `online`, emits airgapped pointer, supports env override
- **B.4** ✅ — version surfaces bumped to `v0.9.1`
- **C.1** ✅ — README, `docs/getting-started.md`, `docs/contributing.md` updated
- **C.2** ✅ — website install docs updated with dual-channel story
- **C.3** ✅ — spec references updated to dual-channel contract

**Open:**
- **D.1** ⬜ — Full validation run: `cargo fmt --all --check`, `cargo check`, `cargo test`, `npm pack --dry-run`, confirm no `slim` references remain.
- **D.2** ⬜ — Push branch, open PR referencing `bge-small-dual-release-channels`, collect sign-off.

**Next gate:** D.1 and D.2. Once both close, `bge-small-dual-release-channels` is ready to archive.
