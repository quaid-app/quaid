# GigaBrain Release Checklist

Zapp must review and sign off on every item before a public release ships.

---

## Asset names

Verify the following artifacts are attached to the GitHub Release:

- [ ] `gbrain-darwin-arm64` — macOS Apple Silicon binary
- [ ] `gbrain-darwin-arm64.sha256` — SHA-256 checksum for macOS Apple Silicon binary
- [ ] `gbrain-darwin-x86_64` — macOS Intel binary
- [ ] `gbrain-darwin-x86_64.sha256` — SHA-256 checksum for macOS Intel binary
- [ ] `gbrain-linux-x86_64` — Linux x86_64 static binary
- [ ] `gbrain-linux-x86_64.sha256` — SHA-256 checksum for Linux x86_64 binary
- [ ] `gbrain-linux-aarch64` — Linux ARM64 static binary
- [ ] `gbrain-linux-aarch64.sha256` — SHA-256 checksum for Linux ARM64 binary

No other binary names are covered by public install docs. If names in the workflow diverge, fix the docs before shipping.

---

## Checksum wording

- [ ] Each `.sha256` file uses standard format: `<hex-digest>  <filename>` (two-space separator)
- [ ] The public verification command matches the file format exactly:
  ```bash
  shasum -a 256 --check "gbrain-<platform>.sha256"
  ```
- [ ] Checksum verification is presented as required, not optional

---

## Install guidance

- [ ] GitHub Releases binaries and build-from-source are the **only** supported install paths in this release
- [ ] Install docs reference artifact names that match the release workflow exactly (`gbrain-<platform>`)
- [ ] Build-from-source instructions list the current stable Rust toolchain requirement and no other system dependencies
- [ ] No install command that is not yet implemented appears anywhere in README or the docs site without a clear **planned — not available yet** label

---

## Deferred distribution channels

The following channels are **not supported** in this release. Confirm none appear as current options:

- [ ] `npm install -g gigabrain` — not available; label as planned follow-on if mentioned
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
