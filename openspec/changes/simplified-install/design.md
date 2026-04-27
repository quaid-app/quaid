## Context

Quaid is preparing a `v0.9.0` test release. Pre-built binaries are produced for four
targets: `darwin-arm64`, `darwin-x86_64`, `linux-x86_64`, `linux-aarch64`. GitHub Releases
assets follow the pattern `quaid-<platform>` (binary) and `quaid-<platform>.sha256`
(checksum).

This change adds a live shell installer for the test release, plus npm package scaffolding
and publish automation that must not break the release flow when `NPM_TOKEN` is absent.

---

## Decisions

### 1. curl installer: POSIX sh, no bash-isms

**Decision:** `scripts/install.sh` uses `#!/usr/bin/env sh` and targets POSIX sh compatibility.
No `[[`, no `local`, no `$'...'` syntax.

**Rationale:** macOS ships with `zsh` as default but `sh` is dash-compatible. Linux containers
often have only `busybox sh`. The installer should work in every Unix environment without
assuming bash.

### 2. Version resolution: GitHub API + env override

**Decision:** When `QUAID_VERSION` is not set, the installer queries the GitHub Releases API
(`https://api.github.com/repos/quaid-app/quaid/releases/latest`) and parses the `tag_name`
field with `sed`. When `QUAID_VERSION` is set, it is used directly.

**Rationale:** Users who want a specific version can pin it. CI and reproducible installs
benefit from explicit pinning. Default "latest" is the right behaviour for one-liners.

`curl` and `jq`/`sed` are the only required tools. Since `jq` may not be present, version
extraction uses `grep` + `sed` on the raw JSON string.

### 3. Default install directory: `~/.local/bin`, no sudo

**Decision:** The installer writes to `${QUAID_INSTALL_DIR:-$HOME/.local/bin}`. It creates
the directory if it does not exist. It prints a PATH hint if the directory is not in `$PATH`.
It does not attempt `sudo` unless `QUAID_INSTALL_DIR` points to a root-owned path.

**Rationale:** Developer tools should not require root. `~/.local/bin` is the XDG-standard
user binary directory and is in `$PATH` on modern Linux distros and macOS with typical
shell configs.

### 4. Checksum verification: sidecar `.sha256` file

**Decision:** The installer downloads `quaid-<platform>.sha256` alongside the binary and
verifies with `shasum -a 256 --check` (macOS) or `sha256sum --check` (Linux), detected via
`uname -s`.

**Rationale:** Follows the same pattern already documented in the README manual install
instructions. Checksum files are already uploaded as release assets. No additional infra needed.

### 5. npm package: postinstall download, no binary bundled

**Decision:** The npm package (`packages/quaid-npm/`) contains only:
- `package.json` — package metadata, bin entry, postinstall hook
- `scripts/postinstall.js` — Node.js script using only built-in modules (`https`, `fs`, `crypto`, `path`, `os`, `child_process`)
- `bin/.gitkeep` — placeholder; actual binary written here at install time

The binary is never committed to the package or bundled in the tarball.

**Rationale:** The binary is ~90MB. Bundling it in the npm tarball would make `npm install`
slow for all users and require separate packages per platform (like `@esbuild/darwin-arm64`).
The postinstall download approach keeps the tarball tiny (<10KB) and requires no per-platform
package split.

### 6. npm postinstall.js: pure Node.js built-ins only

**Decision:** `postinstall.js` uses only Node.js built-in modules. No `node-fetch`, no
`axios`, no dependencies.

**Rationale:** The postinstall script runs before `node_modules` is fully available. External
dependencies would create a chicken-and-egg problem.

### 7. npm publish CI: trigger on semver tags, no-op without token

**Decision:** `.github/workflows/publish-npm.yml` triggers on `push` events with tags
matching `v[0-9]*.[0-9]*.[0-9]*` (same pattern as `release.yml`). It `cd`s into
`packages/quaid-npm/`, runs `npm version` to sync the version from the git tag (with
`--allow-same-version` for idempotency), validates the package structure via `npm pack
--dry-run`, and only runs `npm publish --access public` when `NPM_TOKEN` is present.
If the secret is absent, the workflow emits a notice and exits successfully.

**Rationale:** The tag pattern must match `release.yml` to avoid triggering on non-semver
tags. `--allow-same-version` prevents failure when the tag matches the existing
`package.json` version (e.g., both are `0.9.0`). The dry-run pack step validates the
package structure without requiring registry credentials, keeping the workflow fully
testable even when `NPM_TOKEN` is intentionally absent for the shell-first `v0.9.0`
rollout.

### 8. `scripts/install.sh` hosted on `main` branch

**Decision:** The canonical one-liner references the `main` branch raw URL:
`https://raw.githubusercontent.com/quaid-app/quaid/main/scripts/install.sh`

The script is also uploaded as a release asset for reproducible installs against a specific
version.

**Rationale:** `main` always points to the latest released script, which fetches the latest
binary. Users who want reproducibility can download the script directly from a release asset.

---

## Implementation Details

### `scripts/install.sh` structure

```
1. Detect OS (uname -s): Linux | Darwin | other → error
2. Detect arch (uname -m): x86_64 | aarch64 | arm64 → normalise arm64 to aarch64
3. Map OS+arch → platform string (linux-x86_64 | linux-aarch64 | darwin-x86_64 | darwin-arm64)
4. Resolve version: $QUAID_VERSION or GitHub API latest
5. Construct URLs: binary URL + checksum URL
6. Set INSTALL_DIR: $QUAID_INSTALL_DIR or $HOME/.local/bin
7. mkdir -p $INSTALL_DIR
8. curl download binary to $INSTALL_DIR/quaid.tmp
9. curl download checksum to $INSTALL_DIR/quaid.sha256.tmp
10. Verify: cd into INSTALL_DIR, rename files, run sha256 check
11. chmod +x $INSTALL_DIR/quaid
12. Cleanup temp files
13. Print success message + PATH hint if needed
14. $INSTALL_DIR/quaid version (smoke test)
15. Print QUAID_DB tip (see below)
```

#### QUAID_DB post-install tip

The binary already supports `QUAID_DB` as an env var (`#[arg(long, env = "QUAID_DB", global = true)]`
in `src/main.rs`). Without it, the default database path is `./memory.db` (current working
directory), which is unhelpful for a globally-installed CLI.

The installer prints but does **not** write to any shell config file:

```
Tip: Set QUAID_DB in your shell profile to avoid passing --db on every command:
  echo 'export QUAID_DB="$HOME/memory.db"' >> ~/.zshrc
  # or for bash:
  echo 'export QUAID_DB="$HOME/memory.db"' >> ~/.bashrc
```

The npm postinstall script prints the same tip after a successful binary install.

### `packages/quaid-npm/package.json` shape

```json
{
  "name": "quaid",
  "version": "0.9.0",
  "description": "Quaid — personal AI memory CLI",
  "bin": { "quaid": "bin/quaid" },
  "scripts": { "postinstall": "node scripts/postinstall.js" },
  "engines": { "node": ">=16" },
  "os": ["darwin", "linux"],
  "cpu": ["x64", "arm64"],
  "files": ["bin/.gitkeep", "scripts/postinstall.js"],
  "license": "MIT"
}
```

### `packages/quaid-npm/scripts/postinstall.js` logic

```
1. Map process.platform (darwin/linux) + process.arch (x64/arm64) → platform string
2. Read package version from ../package.json → derive release tag (v<version>)
3. Construct binary URL: https://github.com/quaid-app/quaid/releases/download/v<ver>/quaid-<platform>
4. Construct checksum URL: <binary_url>.sha256
5. Download binary to bin/quaid (or bin/quaid.exe on win32) using https.get with redirect follow
6. Download checksum file
7. Compute SHA-256 of downloaded binary using crypto.createHash('sha256')
8. Compare. If mismatch: delete binary, process.exit(1) with clear error message
9. fs.chmodSync(binaryPath, 0o755)
10. console.log('quaid installed successfully')
```

Graceful failure: if the download fails (network error, unsupported platform), print a
helpful message pointing to the GitHub Releases manual install instructions and exit 0 (not
1) so that `npm install` does not fail the user's overall install — they just won't have the
binary.

### `.github/workflows/publish-npm.yml` structure

```yaml
name: Publish npm package
on:
  push:
    tags: ['v[0-9]*.[0-9]*.[0-9]*']
jobs:
  publish:
    runs-on: ubuntu-latest
    env:
      NPM_TOKEN: ${{ secrets.NPM_TOKEN }}
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: '20'
          registry-url: 'https://registry.npmjs.org'
      - name: Sync version from tag
        run: |
          TAG="${GITHUB_REF_NAME#v}"
          cd packages/quaid-npm
          npm version "$TAG" --no-git-tag-version --allow-same-version
      - name: Skip when token is absent
        if: env.NPM_TOKEN == ''
        run: echo "::notice::NPM_TOKEN is not configured; skipping npm publish for this release."
      - name: Validate package (dry-run)
        working-directory: packages/quaid-npm
        run: npm pack --dry-run
      - name: Publish
        if: env.NPM_TOKEN != ''
        run: npm publish --access public
        working-directory: packages/quaid-npm
        env:
          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
```

---

## Files Created / Modified

| File | Action |
|------|--------|
| `scripts/install.sh` | Create — POSIX curl installer |
| `packages/quaid-npm/package.json` | Create — npm package metadata |
| `packages/quaid-npm/scripts/postinstall.js` | Create — binary download + verify |
| `packages/quaid-npm/bin/.gitkeep` | Create — placeholder for installed binary |
| `packages/quaid-npm/README.md` | Create — npm package readme |
| `.github/workflows/publish-npm.yml` | Create — automated npm publish on tag |
| `README.md` | Update — shell installer live, npm rollout staged |
| `website/src/content/docs/guides/install.md` | Update — shell-first test release guidance |
| `docs/getting-started.md` | Update — install table + rollout guidance |
