# Quaid Release Checklist

Zapp must review and sign off on every item before a public release ships.

---

## Asset names

Every release publishes **8 channel-suffixed binaries** (`airgapped` + `online` × 4 platforms)
plus **8 matching checksums** and `install.sh`. Verify every artifact is attached:

### airgapped channel (embedded BGE-small, no network required)
- [ ] `quaid-darwin-arm64-airgapped` — macOS Apple Silicon binary
- [ ] `quaid-darwin-arm64-airgapped.sha256`
- [ ] `quaid-darwin-x86_64-airgapped` — macOS Intel binary
- [ ] `quaid-darwin-x86_64-airgapped.sha256`
- [ ] `quaid-linux-x86_64-airgapped` — Linux x86_64 static binary
- [ ] `quaid-linux-x86_64-airgapped.sha256`
- [ ] `quaid-linux-aarch64-airgapped` — Linux ARM64 static binary
- [ ] `quaid-linux-aarch64-airgapped.sha256`

### online channel (downloads/caches BGE model on first semantic use)
- [ ] `quaid-darwin-arm64-online`
- [ ] `quaid-darwin-arm64-online.sha256`
- [ ] `quaid-darwin-x86_64-online`
- [ ] `quaid-darwin-x86_64-online.sha256`
- [ ] `quaid-linux-x86_64-online`
- [ ] `quaid-linux-x86_64-online.sha256`
- [ ] `quaid-linux-aarch64-online`
- [ ] `quaid-linux-aarch64-online.sha256`

### installer
- [ ] `install.sh`

**Total: 17 files (8 binaries + 8 checksums + install.sh).** The release workflow enforces
this count and fails closed if any asset is missing. A release is not shippable if the
workflow did not complete successfully — do not hand-patch partial releases.

The public asset schema is `quaid-<platform>-<channel>` where `<platform>` ∈
`{darwin-arm64, darwin-x86_64, linux-x86_64, linux-aarch64}` and `<channel>` ∈
`{airgapped, online}`. `.github/release-assets.txt` is the canonical manifest shared by
`install.sh` validation, the release workflow, and release-check tests. Do not approve a
release whose checklist, docs, or workflow diverge from this schema.

---

## Checksum wording

- [ ] Each `.sha256` file uses standard format: `<hex-digest>  <filename>` (two-space separator)
- [ ] The public verification command matches the file format exactly:
  ```bash
  shasum -a 256 --check "quaid-<platform>-<channel>.sha256"
  ```
- [ ] Checksum verification is presented as required, not optional

---

## Install guidance

- [ ] GitHub Releases binaries and `install.sh` are the **only** supported install paths in this release
- [ ] Install docs reference artifact names that match the release workflow exactly (`quaid-<platform>-<channel>`)
- [ ] Build-from-source instructions list the current stable Rust toolchain requirement and no other system dependencies
- [ ] No install command that is not yet implemented appears anywhere in README or the docs site without a clear **planned — not available yet** label

---

## Deferred distribution channels

The following channels are **not supported** in this release. Confirm none appear as current options:

- [ ] `npm install -g quaid` — not available; label as planned follow-on if mentioned
- [ ] `curl | sh` one-command installer — not available; label as planned follow-on if mentioned
- [ ] Homebrew tap, winget, or any other package manager entry — not available; label as planned follow-on if mentioned
- [ ] Release notes do not promise npm publishing or a simplified installer in this version

---

## Hard-breaking rename migration gate

This release carries the **quaid-hard-rename** change. The following are manual, non-reversible
user-side changes required to continue using the tool. Confirm each is documented in the release
notes and the upgrade guide before shipping:

- [ ] Release notes open with an explicit **⚠ BREAKING RENAME** callout — not buried in the body
- [ ] Binary name change documented: old binary name is gone; the new binary is `quaid`. Any PATH
  entry, shell alias, or script invoking the old name must be updated manually.
- [ ] MCP tool prefix change documented: all 17 tools renamed from the legacy prefix to `memory_*`.
  Every MCP client config (Claude Code `.mcp.json`, Cursor, etc.) must be updated manually —
  clients will see zero tools until the config references the new names.
- [ ] Env var prefix change documented: all environment variables are now `QUAID_*`. Any shell
  profile lines, CI secrets, or dotenv files using the old prefix must be updated manually.
- [ ] DB migration path documented: existing databases are incompatible. Users must export with
  the old binary, run `quaid init ~/.quaid/memory.db`, then `quaid import <backup/>`.
  No automatic migration is provided.
- [ ] npm package name change documented: the npm package is now `quaid` (not the legacy name).
  Existing global installs of the old package must be uninstalled separately.
- [ ] `packages/quaid-npm/` is committed and tracked; `packages/gbrain-npm/` is fully removed
  from the tree. The publish workflow will fail if `packages/quaid-npm/` is absent.

---

## Release notes

- [ ] Release notes accurately describe what changed in this version
- [ ] Release notes do not promise unsupported distribution channels
- [ ] Release notes include a pointer to the install section of the docs
- [ ] Release notes include a pointer to the migration guide for users upgrading from a pre-rename version

---

## Sign-off

| Role | Owner | Status |
| ---- | ----- | ------ |
| Launch wording and release copy | Zapp | ☐ |
| Hard-breaking rename migration gate | Zapp | ☐ |
| Release workflow assets and checksums | Fry | ☐ |
| Scope confirmed against approved proposal | Leela | ☐ |
