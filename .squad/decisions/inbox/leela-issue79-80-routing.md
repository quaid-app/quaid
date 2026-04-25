# Routing Memo: Issues #79 + #80 → v0.9.7 Release

**Author:** Leela  
**Date:** 2026-04-25  
**Status:** Branch pushed, PR #83 open, pending CI + merge

---

## What happened

Two beta issues arrived from coding-beth (v0.9.6):
- **#79** — `install.sh` HTTP 404 for `darwin-x86_64-airgapped`
- **#80** — compile error `stat.st_mode`: expected `u32`, found `u16` on macOS

Professor's prior review (decisions.md) correctly identified these as a single failure
chain: macOS compilation broke all four macOS release targets → no macOS assets uploaded
→ installer gets 404. The issues are not independent.

---

## Root cause (confirmed)

`src/core/fs_safety.rs:199` assigned `stat.st_mode` (type `u16` on macOS) directly into
`FileStatNoFollow.mode_bits` (type `u32`). All four macOS CI jobs failed. The release job
never ran for macOS. `install.sh` asset naming was correct throughout — nothing to fix there.

---

## What shipped in this release lane

**Commit 1** (prior, on branch): fs_safety.rs cast fix + Cargo.toml bump to 0.9.7 + openspec proposal  
**Commit 2** (this session): contract centralization + CI hardening + proofs

| File | Change |
|------|--------|
| `src/core/fs_safety.rs` | `stat.st_mode as u32` — closes #80 |
| `Cargo.toml` | 0.9.6 → 0.9.7 |
| `packages/gbrain-npm/package.json` | 0.9.6 → 0.9.7 |
| `src/core/inference.rs` | user-agent 0.9.6 → 0.9.7 |
| `.github/workflows/ci.yml` | macOS preflight (cargo check × 4 targets × 2 channels) + seam test step |
| `.github/workflows/release.yml` | release notes body updated |
| `.github/RELEASE_CHECKLIST.md` | canonical 17-file schema (closes #79 contract) |
| `tests/install_release_seam.sh` | seam test: 8 combos, installer + workflow agree |
| `tests/release_asset_parity.sh` | static parity: install.sh ↔ release.yml ↔ checklist |

---

## PR

**PR #83** — `release/v0.9.7` → `main`  
https://github.com/macro88/gigabrain/pull/83

---

## Merge gates (D-R79-6)

| Gate | Status |
|------|--------|
| macOS build fixed (`stat.st_mode as u32`) | ✅ committed |
| Contract centralized (`gbrain-<platform>-<channel>` everywhere) | ✅ committed |
| Manifest proof (`release_asset_parity.sh`) | ✅ committed |
| Installer proof (`install_release_seam.sh`) | ✅ committed |
| Reviewer surface truthful (RELEASE_CHECKLIST.md 17-file schema) | ✅ committed |
| Real release evidence (CI green + v0.9.7 tag + 17 assets present) | ⏳ pending CI |

**Do not tag until CI is green.** The `release-macos-preflight` job in ci.yml is the
primary signal. If it passes, the macOS build is repaired and all 6 Professor gates are met.

---

## Post-merge release procedure

1. Wait for CI green on PR #83
2. Merge PR #83 to `main`
3. Create annotated tag:
   ```
   git tag -a v0.9.7 -m "v0.9.7 — macOS build fix + asset contract centralization

   Fixes #79 and #80. All four macOS targets now build. install.sh resolves
   gbrain-<platform>-<channel> against a complete, verified release manifest.
   "
   git push origin v0.9.7
   ```
4. Monitor release workflow; verify 17 artifacts (8 binaries + 8 checksums + install.sh)
5. Close issues #79 and #80

---

## Routing

- **Professor** — verify CI result satisfies D-R79-6 gate 6 before approving merge
- **Fry** — FYI only; branch work is complete
- **macro88** — merge gate is CI green; nothing manual needed after merge except the tag push

---

## Decisions recorded

No new decisions beyond what Professor already locked in decisions.md (D-R79-1 through D-R79-6).
This memo is the execution trace.
