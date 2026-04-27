# Simplified Install — Implementation Checklist

**Scope:** shell-first `v0.9.0` installer rollout, npm package scaffolding, token-safe CI publish workflow, docs updates.

---

## Phase A — curl one-liner installer

- [x] A.1 Create `scripts/install.sh`: POSIX sh script with `#!/usr/bin/env sh`. Detect OS via `uname -s` (Linux/Darwin) and arch via `uname -m` (x86_64/aarch64/arm64). Map to platform string (`linux-x86_64`, `linux-aarch64`, `darwin-x86_64`, `darwin-arm64`). Exit with clear message for unsupported platforms.

- [x] A.2 Add version resolution to `scripts/install.sh`: when `$QUAID_VERSION` is unset, query `https://api.github.com/repos/quaid-app/quaid/releases/latest` with `curl -fsSL` and extract `tag_name` using `grep` + `sed` (no `jq` dependency). Use `$QUAID_VERSION` directly when set.

- [x] A.3 Add download + verify logic to `scripts/install.sh`: download binary and `.sha256` sidecar to a temp directory. Verify using `shasum -a 256 --check` (Darwin) or `sha256sum --check` (Linux). On mismatch: delete temp files, print error, exit 1.

- [x] A.4 Add install logic to `scripts/install.sh`: install to `${QUAID_INSTALL_DIR:-$HOME/.local/bin}`, create directory if absent, `chmod +x`, run `quaid version` as smoke test. Print PATH hint if install dir is not in `$PATH`. After the smoke test, print the `QUAID_DB` tip: `Tip: Set QUAID_DB in your shell profile to avoid passing --db on every command:` followed by the `echo 'export QUAID_DB="$HOME/memory.db"'` example for both zsh and bash. Do not modify any shell config files.

- [x] A.5 Upload `scripts/install.sh` as a release asset: in `.github/workflows/release.yml` (or equivalent release workflow), add a step to attach `scripts/install.sh` to the GitHub Release using `gh release upload`.

---

## Phase B — npm package

- [x] B.1 Create `packages/quaid-npm/` directory structure:
  - `packages/quaid-npm/package.json` — name `quaid`, version `0.9.0`, bin entry `bin/quaid`, postinstall hook, engines `node>=18`, os `[darwin, linux]`, cpu `[x64, arm64]`, files list
  - `packages/quaid-npm/bin/quaid` — committed shell wrapper that execs `quaid.bin` (downloaded by postinstall) or prints manual-install guidance
  - `packages/quaid-npm/README.md` — brief description + `npm install -g quaid` + `quaid init` quick start

- [x] B.2 Create `packages/quaid-npm/scripts/postinstall.js`: pure Node.js built-ins only. Map `process.platform` + `process.arch` → platform string; derive version from `package.json`; construct GitHub Releases download URL; download binary with `https.get` following redirects (60s timeout); download checksum; verify SHA-256 with `crypto.createHash('sha256')`; write to `bin/quaid.bin`; `fs.chmodSync` 0o755. After success, print the `QUAID_DB` tip (same wording as the sh installer). On unsupported platform or network failure: print helpful message pointing to manual install URL and exit 0 (do not fail the overall `npm install`).

- [x] B.3 Add `packages/quaid-npm/bin/quaid.bin` and `quaid.download` to `.gitignore` (downloaded binary must not be committed; the wrapper script is tracked).

- [x] B.4 Create `.github/workflows/publish-npm.yml`: trigger on `push` with `tags: ['v[0-9]*.[0-9]*.[0-9]*']` (matching `release.yml`). Steps: checkout, setup-node with `registry-url: https://registry.npmjs.org`, sync version from git tag via `npm version $TAG --no-git-tag-version --allow-same-version`, validate with `npm pack --dry-run`, and only run `npm publish --access public` when `NPM_TOKEN` is present; otherwise emit a notice and succeed.

- [x] B.5 Document `NPM_TOKEN` secret requirement in `docs/contributing.md` (add a "Release process" or "Secrets" section noting that `NPM_TOKEN` must be set in repo secrets before the first public publish, and that missing the secret should skip npm publication rather than fail the release workflow).

---

## Phase C — docs updates

- [x] C.1 Update `README.md` install table: change the curl installer row to `✅ Available` with `curl -fsSL https://raw.githubusercontent.com/quaid-app/quaid/main/scripts/install.sh | sh`, and change the npm row to staged / not yet public for the `v0.9.0` test cycle. Replace the deferred-distribution callout with rollout guidance that explains shell-first testing.

- [x] C.2 Update `website/src/content/docs/guides/install.md`: replace the deferred curl section with live install instructions. Show the one-liner + env override examples, and explain that npm packaging is implemented but public publication remains gated until after shell-installer testing and `NPM_TOKEN` configuration.

- [x] C.3 Update `docs/getting-started.md`: update install options table to show the curl installer as available, the npm path as staged, and replace the deferred callout with shell-first `v0.9.0` rollout guidance.

---

## Phase D — verification

- [x] D.1 Smoke test `scripts/install.sh` locally against the `v0.9.0` release asset shape: run the full script when a matching release is available, confirm `quaid version` succeeds.

- [x] D.2 Test npm postinstall locally: `cd packages/quaid-npm && npm install` (or `npm pack` then `npm install -g quaid-0.9.10.tgz`). Confirm `quaid version` works and binary is in `bin/quaid`.
  - ~~Product gap: `v0.9.0` is not an actual GitHub Release~~ — RESOLVED: release assets are now present and package version is currently `0.9.10`.
  - `npm pack --dry-run` validated in CI (run 24516842061): 4 files, 2.4KB tarball, binary NOT packed. Package shape confirmed.
  - Asset-name alignment verified: `postinstall.js` platform→asset mapping matches all 4 release asset names exactly.
  - **Windows evidence (confirmed 2026-04-27):** local `npm install` on `win32/x64` fails with `EBADPLATFORM`, expected due to package `os`/`cpu` restrictions (`darwin,linux` + `x64,arm64`).
  - **Linux evidence (confirmed 2026-04-27):** `npm install` succeeded on Ubuntu and `postinstall.js` reported `Installed quaid-linux-x86_64-online (online channel) from GitHub Releases.`
  - **Closure evidence (Ubuntu, 2026-04-27):** `ls -l bin/quaid.bin` showed executable binary present (`-rwxrwxrwx`, size `13905840`), and `./bin/quaid version` returned `quaid 0.9.10`.

- [x] D.3 Run `npm pack --dry-run` from `packages/quaid-npm/` and confirm `bin/quaid` is NOT listed in the packed files (only `bin/.gitkeep` and `scripts/postinstall.js`).

- [x] D.4 Test error paths:
  - Set `QUAID_VERSION=v0.0.0-nonexistent` and run `install.sh` — validated in WSL after normalizing `scripts/install.sh` to LF; installer prints a clean download error and exits 1.
  - Corrupt the downloaded binary checksum and confirm the installer exits 1 before placing the binary — validated in WSL with a fake `curl` earlier in `PATH`; installer exits 1 and does not install `quaid`.
  - Run `npm install` in an environment with no internet — validated with a platform-aware Node harness (`linux/x64` override + mocked `ENOTFOUND`); `postinstall.js` prints the manual-install fallback and exits 0.

- [x] D.5 Verify `.github/workflows/publish-npm.yml` token-guard behavior: confirmed via real CI execution against v0.9.0 tag (run 24516842061, 2026-04-16T14:46:25Z).
  - **Token-absent path (proven):** "Skip publish when token is absent" step executed, logged `::notice::NPM_TOKEN is not configured; skipping npm publish for this release.` "Publish to npm" step was correctly skipped via `if: env.NPM_TOKEN != ''` guard.
  - **Package validation (proven):** `npm pack --dry-run` succeeded in CI — `quaid@0.9.0`, 4 files (README.md, bin/.gitkeep, package.json, scripts/postinstall.js), 2.4KB tarball, binary excluded.
  - **Token-present path (by-design deferred):** NPM_TOKEN secret is not configured in the repo. Positive-path publish is gated behind shell-installer validation and explicit secret configuration. Structural guard verified through negative evidence + workflow code review.
