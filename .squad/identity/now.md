updated_at: 2026-04-18T00:00:00Z
focus_area: simplified-install — v0.9.0 shell-first rollout
active_issues: [D.2-npm-postinstall-needs-supported-platform]
active_branch: simplified-install
---

# What We're Focused On

**Active change:** `simplified-install` — shell-first `v0.9.0` installer rollout.

Implementation is complete. One verification item remains environment-blocked.

**Done:**
- **A** ✅ — `scripts/install.sh` (POSIX, platform detect, SHA-256 verify, `GBRAIN_DB` tip)
- **B** ✅ — `packages/gbrain-npm/` scaffolding + `postinstall.js` + `.github/workflows/publish-npm.yml`
- **C** ✅ — README, `website/…/install.md`, `docs/getting-started.md` updated (shell-first, npm staged)
- **D.1** ✅ — `install.sh` smoke-tested against `v0.9.0` release asset shape
- **D.3** ✅ — `npm pack --dry-run` confirms binary not packed
- **D.4** ✅ — error paths validated (bad version, bad checksum, no-internet postinstall exit-0)
- **D.5** ✅ — `publish-npm.yml` token-guard verified via real CI execution against v0.9.0 tag (run 24516842061). Skip-notice printed, pack validated, publish skipped when token absent.

**Blocked / needs supported platform:**
- **D.2** ⚠️ — npm postinstall end-to-end test still needs a macOS/Linux machine with Node.js. The v0.9.0 release now exists with all assets, and asset-name alignment is verified — but nobody has run the actual download+verify cycle through postinstall.js yet.

**Next gate:** D.2 can close once a macOS/Linux runner with Node.js is available to exercise the postinstall cycle against the live v0.9.0 release.
