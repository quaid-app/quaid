---
name: quaid-upgrade
description: |
  Agent-guided binary and skill updates: version check, download, SHA-256 verify,
  post-upgrade validation, rollback on failure.
min_binary_version: "0.3.0"
---

# Upgrade Skill

## Overview

The upgrade skill guides an agent through safely replacing the `quaid` binary and
refreshing bundled skill files. It is conservative by default: it keeps a `.bak` copy
of the previous binary, verifies checksums before replacing, and runs `quaid validate --all`
before declaring success. If validation fails, it rolls back automatically.

---

## Commands

```bash
quaid version                          # print current binary version
quaid validate --all --json            # post-upgrade integrity check
quaid skills list --json               # list active skills after upgrade
quaid skills doctor --json             # verify skill resolution and hashes
```

---

## Upgrade Workflow

### Step 1 — Check current version

```bash
quaid version
```

Output: `quaid 0.2.0 (commit abc1234)`

Record `current_version` for comparison.

### Step 2 — Fetch latest release metadata

Query the GitHub Releases API (no authentication required for public repos):

```
GET https://api.github.com/repos/quaid-app/quaid/releases/latest
```

Extract from response:
- `tag_name` → `latest_version` (e.g., `v0.3.0`)
- `assets[].browser_download_url` → download URL matching the current platform
- `assets[].name` — find the `.sha256` checksum asset alongside the binary

**Platform asset naming convention:**

| Platform | Asset filename |
|----------|---------------|
| Linux x86_64 | `quaid-x86_64-unknown-linux-musl` |
| Linux ARM64 | `quaid-aarch64-unknown-linux-musl` |
| macOS x86_64 | `quaid-x86_64-apple-darwin` |
| macOS ARM64 | `quaid-aarch64-apple-darwin` |

If no asset matches the current platform, abort and report: `Error: no release asset for <platform>`.

### Step 3 — Compare versions

If `current_version == latest_version`:
- Print: `quaid is already up to date (v<version>). No action taken.`
- Exit.

If `latest_version` is older than `current_version` (downgrade):
- Warn: `Warning: release v<latest> is older than installed v<current>. Skipping.`
- Exit unless the agent was explicitly instructed to downgrade.

### Step 4 — Download binary and checksum

```bash
curl -fL "<binary_url>" -o quaid.new
curl -fL "<sha256_url>" -o quaid.new.sha256
```

Do NOT replace the existing binary yet.

### Step 5 — Verify checksum

```bash
sha256sum -c quaid.new.sha256
```

If verification fails:
- Delete `quaid.new` and `quaid.new.sha256`
- Abort: `Error: checksum verification failed. Downloaded binary is corrupt or tampered.`
- Do NOT proceed.

### Step 6 — Back up existing binary

```bash
cp "$(which quaid)" "$(which quaid).bak"
```

If `$(which quaid)` is not writable (e.g., installed in `/usr/local/bin` without sudo):
- Report the path and instruct the user to run the replacement step with elevated privileges.
- Provide the exact command: `sudo cp quaid.new $(which quaid) && sudo chmod +x $(which quaid)`

### Step 7 — Replace binary

```bash
chmod +x quaid.new
mv quaid.new "$(which quaid)"
```

### Step 8 — Post-upgrade validation

```bash
quaid version          # confirm new version reports correctly
quaid validate --all --json
```

If `validate --all` exits with code 1 (violations found):
- **Automatically roll back** (see Rollback Procedure below).
- Report: `Upgrade validation failed. Rolled back to v<previous>. Run 'quaid validate --all' to inspect violations.`

If `validate --all` exits with code 0:
- Print: `Upgrade complete. quaid v<new_version> is active.`

### Step 9 — Update skills

If the new release bundles updated skill files (check release notes for mention of `SKILL.md` changes):

```bash
quaid skills doctor --json
```

Review the output for skills where `embedded_hash != previous_hash`. The new embedded
skills are automatically active for the embedded resolution tier. External overrides
(`~/.quaid/skills/` or `./skills/`) take precedence and are NOT automatically updated
— the agent should alert the user if an external override exists for a skill that changed.

---

## Rollback Procedure

If post-upgrade validation fails or the new binary does not start:

```bash
mv "$(which quaid)" "$(which quaid).failed"
cp "$(which quaid).bak" "$(which quaid)"
quaid version    # confirm rollback succeeded
```

If the rollback binary also fails to start:
- The `.bak` file is preserved as `quaid.failed` is moved aside.
- Report: `Critical: rollback binary also failed. Manual recovery required. Backup at: $(which quaid).bak`

---

## Version Pinning Rules

Skills declare a minimum binary version via the `min_binary_version` frontmatter field.
The upgrade skill enforces this:

1. After upgrade, run `quaid skills doctor --json`.
2. For each skill, check `min_binary_version` against the installed version.
3. If any skill requires a higher version than installed, report:
   `Warning: skill <name> requires quaid >= <version>. Current: <installed>. Upgrade to satisfy.`

The binary will still run; this is a warning, not a hard block.

---

## Failure Modes

| Condition | Behaviour |
|-----------|-----------|
| GitHub API unreachable | Abort: `Error: cannot reach GitHub Releases API. Check network.` |
| No matching platform asset | Abort with platform name in error message |
| Checksum mismatch | Delete downloads, abort. Never replace binary. |
| Insufficient write permissions | Print manual replacement command; do not attempt unattended escalation |
| `validate --all` fails post-upgrade | Automatic rollback to `.bak` binary |
| `.bak` missing at rollback time | Report critical error; preserve `.failed` binary for forensics |
