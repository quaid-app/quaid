# GigaBrain Release Checklist

Zapp must review and sign off on every item before a public release ships.

---

## Asset names

Every release publishes **8 channel-suffixed binaries** (`airgapped` + `online` × 4 platforms)
plus **8 matching checksums** and `install.sh`. Verify every artifact is attached:

### airgapped channel (embedded BGE-small, no network required)
- [ ] `gbrain-darwin-arm64-airgapped` — macOS Apple Silicon binary
- [ ] `gbrain-darwin-arm64-airgapped.sha256`
- [ ] `gbrain-darwin-x86_64-airgapped` — macOS Intel binary
- [ ] `gbrain-darwin-x86_64-airgapped.sha256`
- [ ] `gbrain-linux-x86_64-airgapped` — Linux x86_64 static binary
- [ ] `gbrain-linux-x86_64-airgapped.sha256`
- [ ] `gbrain-linux-aarch64-airgapped` — Linux ARM64 static binary
- [ ] `gbrain-linux-aarch64-airgapped.sha256`

### online channel (downloads/caches BGE model on first semantic use)
- [ ] `gbrain-darwin-arm64-online`
- [ ] `gbrain-darwin-arm64-online.sha256`
- [ ] `gbrain-darwin-x86_64-online`
- [ ] `gbrain-darwin-x86_64-online.sha256`
- [ ] `gbrain-linux-x86_64-online`
- [ ] `gbrain-linux-x86_64-online.sha256`
- [ ] `gbrain-linux-aarch64-online`
- [ ] `gbrain-linux-aarch64-online.sha256`

### installer
- [ ] `install.sh`

**Total: 17 files (8 binaries + 8 checksums + install.sh).** The release workflow enforces
this count and fails closed if any asset is missing. A release is not shippable if the
workflow did not complete successfully — do not hand-patch partial releases.

The public asset schema is `gbrain-<platform>-<channel>` where `<platform>` ∈
`{darwin-arm64, darwin-x86_64, linux-x86_64, linux-aarch64}` and `<channel>` ∈
`{airgapped, online}`. `.github/release-assets.txt` is the canonical manifest shared by
`install.sh` validation, the release workflow, and release-check tests. Do not approve a
release whose checklist, docs, or workflow diverge from this schema.

---

## Checksum wording

- [ ] Each `.sha256` file uses standard format: `<hex-digest>  <filename>` (two-space separator)
- [ ] The public verification command matches the file format exactly:
  ```bash
  shasum -a 256 --check "gbrain-<platform>-<channel>.sha256"
  ```
- [ ] Checksum verification is presented as required, not optional

---

## Install guidance

- [ ] GitHub Releases binaries and `install.sh` are the **only** supported install paths in this release
- [ ] Install docs reference artifact names that match the release workflow exactly (`gbrain-<platform>-<channel>`)
- [ ] Build-from-source instructions list the current stable Rust toolchain requirement and no other system dependencies
- [ ] No install command that is not yet implemented appears anywhere in README or the docs site without a clear **planned — not available yet** label

---

## Deferred distribution channels

The following channels are **not supported** in this release. Confirm none appear as current options:

- [ ] `npm install -g gbrain` — not available; label as planned follow-on if mentioned
- [ ] `curl | sh` one-command installer — not available; label as planned follow-on if mentioned
- [ ] Homebrew tap, winget, or any other package manager entry — not available; label as planned follow-on if mentioned
- [ ] Release notes do not promise npm publishing or a simplified installer in this version

---

## Release notes

- [ ] Release notes accurately describe what changed in this version
- [ ] Release notes do not promise unsupported distribution channels
- [ ] Release notes include a pointer to the install section of the docs

---

## Sign-off

| Role | Owner | Status |
| ---- | ----- | ------ |
| Launch wording and release copy | Zapp | ☐ |
| Release workflow assets and checksums | Fry | ☐ |
| Scope confirmed against approved proposal | Leela | ☐ |
