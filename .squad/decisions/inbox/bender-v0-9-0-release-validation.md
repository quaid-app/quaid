# Decision: v0.9.0 Release Lane Validation

**Author:** Bender (Tester)
**Date:** 2026-04-18
**Status:** APPROVED with one open item

## Context

Zapp pushed the v0.9.0 tag (commit c1f572b) which triggered both the Release and Publish npm workflows. This validation checks the real CI execution and release assets against the simplified-install proposal.

## Evidence

### Release workflow (run 24516840337)
- **Conclusion:** success
- **Build jobs:** All 4 platform targets built successfully
  - `aarch64-apple-darwin` on macos-latest ✅
  - `x86_64-apple-darwin` on macos-latest ✅
  - `x86_64-unknown-linux-musl` on ubuntu-latest ✅ (static link verified)
  - `aarch64-unknown-linux-musl` on ubuntu-latest ✅ (static link verified)
- **Release job:** All 8 binary+checksum artifacts verified present, checksums re-verified post-download, release created, install.sh uploaded as 9th asset

### Release assets (v0.9.0)
| Asset | Size | State |
|-------|------|-------|
| gbrain-darwin-arm64 | 7.7MB | uploaded |
| gbrain-darwin-arm64.sha256 | 86B | uploaded |
| gbrain-darwin-x86_64 | 8.5MB | uploaded |
| gbrain-darwin-x86_64.sha256 | 87B | uploaded |
| gbrain-linux-aarch64 | 7.9MB | uploaded |
| gbrain-linux-aarch64.sha256 | 87B | uploaded |
| gbrain-linux-x86_64 | 9.5MB | uploaded |
| gbrain-linux-x86_64.sha256 | 86B | uploaded |
| install.sh | 3.7KB | uploaded |

Release is not draft, not prerelease (correct: v0.9.0 has no hyphen).

### Publish npm workflow (run 24516842061)
- **Conclusion:** success
- **Token-absent path:** "Skip publish when token is absent" step executed, logged `::notice::NPM_TOKEN is not configured; skipping npm publish for this release.`
- **Package validation:** `npm pack --dry-run` succeeded — `gbrain@0.9.0`, 4 files (README.md, bin/.gitkeep, package.json, scripts/postinstall.js), 2.4KB tarball. Binary NOT in tarball.
- **Publish step:** Correctly SKIPPED (`if: env.NPM_TOKEN != ''` guard worked)

### Asset-name alignment
- `install.sh` platform→asset mapping matches all 4 release assets ✅
- `postinstall.js` platform→asset mapping matches all 4 release assets ✅
- Download URLs resolve correctly against real release ✅

## Decisions

### D.5 — CLOSED ✅
Token-guard behavior proven through real CI execution. The task's acceptance criteria (workflow publishes only when NPM_TOKEN present; otherwise exits with notice) are satisfied by direct evidence. Positive-path publish is by-design deferred until NPM_TOKEN is configured.

### D.2 — REMAINS OPEN
The "v0.9.0 is not a real release" blocker is resolved. Asset-name alignment is verified. Package shape is validated. But the end-to-end postinstall download+verify cycle has not been exercised on a macOS or Linux machine. This needs a supported-platform runner with Node.js to close.

## Observations

- Release binary sizes are 7.7–9.5MB, not ~90MB as the proposal estimated. The 90MB figure likely assumed embedded model weights; actual release builds appear to use the `online-model` feature or simply don't embed the full model.
- Node.js 20 deprecation warnings in both workflows. Actions will force Node.js 24 starting June 2026. Non-blocking but should be tracked.

## Recommendation

Ship confidence is high for the shell-installer and release pipeline. The only gap is D.2's end-to-end npm postinstall test, which is a real verification gap but does not block the shell-first v0.9.0 release posture.
