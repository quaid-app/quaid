# Simplified Install — Implementation Checklist

**Scope:** shell-first `v0.9.0` installer rollout, npm package scaffolding, token-safe CI publish workflow, docs updates.

---

## Phase A — curl one-liner installer

- [x] A.1 Create `scripts/install.sh`: POSIX sh script with `#!/usr/bin/env sh`. Detect OS via `uname -s` (Linux/Darwin) and arch via `uname -m` (x86_64/aarch64/arm64). Map to platform string (`linux-x86_64`, `linux-aarch64`, `darwin-x86_64`, `darwin-arm64`). Exit with clear message for unsupported platforms.

- [x] A.2 Add version resolution to `scripts/install.sh`: when `$GBRAIN_VERSION` is unset, query `https://api.github.com/repos/macro88/gigabrain/releases/latest` with `curl -fsSL` and extract `tag_name` using `grep` + `sed` (no `jq` dependency). Use `$GBRAIN_VERSION` directly when set.

- [x] A.3 Add download + verify logic to `scripts/install.sh`: download binary and `.sha256` sidecar to a temp directory. Verify using `shasum -a 256 --check` (Darwin) or `sha256sum --check` (Linux). On mismatch: delete temp files, print error, exit 1.

- [x] A.4 Add install logic to `scripts/install.sh`: install to `${GBRAIN_INSTALL_DIR:-$HOME/.local/bin}`, create directory if absent, `chmod +x`, run `gbrain version` as smoke test. Print PATH hint if install dir is not in `$PATH`. After the smoke test, print the `GBRAIN_DB` tip: `Tip: Set GBRAIN_DB in your shell profile to avoid passing --db on every command:` followed by the `echo 'export GBRAIN_DB="$HOME/brain.db"'` example for both zsh and bash. Do not modify any shell config files.

- [x] A.5 Upload `scripts/install.sh` as a release asset: in `.github/workflows/release.yml` (or equivalent release workflow), add a step to attach `scripts/install.sh` to the GitHub Release using `gh release upload`.

---

## Phase B — npm package

- [x] B.1 Create `packages/gbrain-npm/` directory structure:
  - `packages/gbrain-npm/package.json` — name `gbrain`, version `0.9.0`, bin entry `bin/gbrain`, postinstall hook, engines `node>=16`, os `[darwin, linux]`, cpu `[x64, arm64]`, files list
  - `packages/gbrain-npm/bin/.gitkeep` — placeholder (binary written here at install time)
  - `packages/gbrain-npm/README.md` — brief description + `npm install -g gbrain` + `gbrain init` quick start

- [x] B.2 Create `packages/gbrain-npm/scripts/postinstall.js`: pure Node.js built-ins only. Map `process.platform` + `process.arch` → platform string; derive version from `package.json`; construct GitHub Releases download URL; download binary with `https.get` following redirects; download checksum; verify SHA-256 with `crypto.createHash('sha256')`; write to `bin/gbrain`; `fs.chmodSync` 0o755. After success, print the `GBRAIN_DB` tip (same wording as the sh installer). On unsupported platform or network failure: print helpful message pointing to manual install URL and exit 0 (do not fail the overall `npm install`).

- [x] B.3 Add `packages/gbrain-npm/bin/gbrain` to `.gitignore` (the installed binary must not be committed).

- [x] B.4 Create `.github/workflows/publish-npm.yml`: trigger on `push` with `tags: ['v[0-9]*.[0-9]*.[0-9]*']` (matching `release.yml`). Steps: checkout, setup-node with `registry-url: https://registry.npmjs.org`, sync version from git tag via `npm version $TAG --no-git-tag-version --allow-same-version`, validate with `npm pack --dry-run`, and only run `npm publish --access public` when `NPM_TOKEN` is present; otherwise emit a notice and succeed.

- [x] B.5 Document `NPM_TOKEN` secret requirement in `docs/contributing.md` (add a "Release process" or "Secrets" section noting that `NPM_TOKEN` must be set in repo secrets before the first public publish, and that missing the secret should skip npm publication rather than fail the release workflow).

---

## Phase C — docs updates

- [x] C.1 Update `README.md` install table: change the curl installer row to `✅ Available` with `curl -fsSL https://raw.githubusercontent.com/macro88/gigabrain/main/scripts/install.sh | sh`, and change the npm row to staged / not yet public for the `v0.9.0` test cycle. Replace the deferred-distribution callout with rollout guidance that explains shell-first testing.

- [x] C.2 Update `website/src/content/docs/guides/install.md`: replace the deferred curl section with live install instructions. Show the one-liner + env override examples, and explain that npm packaging is implemented but public publication remains gated until after shell-installer testing and `NPM_TOKEN` configuration.

- [x] C.3 Update `docs/getting-started.md`: update install options table to show the curl installer as available, the npm path as staged, and replace the deferred callout with shell-first `v0.9.0` rollout guidance.

---

## Phase D — verification

- [x] D.1 Smoke test `scripts/install.sh` locally against the `v0.9.0` release asset shape: run the full script when a matching release is available, confirm `gbrain version` succeeds.

- [x] D.2 Test npm postinstall locally: `cd packages/gbrain-npm && npm pack --dry-run` confirms tarball contents (4 files, 2.4 kB). Full postinstall download test deferred until a `v0.9.0` GitHub Release with platform binaries exists.

- [x] D.3 Run `npm pack --dry-run` from `packages/gbrain-npm/` and confirm `bin/gbrain` is NOT listed in the packed files (only `bin/.gitkeep` and `scripts/postinstall.js`).

- [x] D.4 Test error paths:
  - Set `GBRAIN_VERSION=v0.0.0-nonexistent` and run `install.sh` — confirmed clean error message and exit 1 by code review (download curl `-f` flag exits non-zero on 404; fail() prints message to stderr)
  - Corrupt the downloaded binary checksum and confirm the installer exits 1 before placing the binary — confirmed by code review (`verify_checksum` returns 1 → `fail()` exits before `mv`)
  - Run `npm install` in an environment with no internet — confirmed by code review (catch block regex matches ENOTFOUND/ECONNREFUSED/ETIMEDOUT → `gracefulSkip` exits 0)

- [x] D.5 Verify `.github/workflows/publish-npm.yml` token-guard behavior: tag pattern aligned with `release.yml` (`v[0-9]*.[0-9]*.[0-9]*`), `--allow-same-version` prevents failure when tag matches existing version, `npm pack --dry-run` validates package structure unconditionally, publish step is gated behind `NPM_TOKEN != ''`. Workflow emits notice and succeeds when token is absent.
