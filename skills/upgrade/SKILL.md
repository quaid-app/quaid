---
name: quaid-upgrade
description: |
  Agent-guided binary upgrades: version check, channel detection, download via
  install.sh (or manual asset fetch + SHA-256 verify), and post-upgrade validation.
min_binary_version: "0.3.0"
---

# Upgrade Skill

## Overview

The upgrade skill guides an agent through safely replacing the `quaid` binary.
The fast, supported path is to **re-run the official installer**, which already
handles platform detection, channel selection, download, checksum verification,
and PATH placement. Fall back to a manual download only when the installer
cannot run (no network to the raw script, locked-down environment, etc.).

Skills are embedded in the binary, so refreshing the binary refreshes the
default skills automatically. User or project overrides under
`~/.quaid/skills/` and `./skills/` are NOT touched by an upgrade — see
[Refreshing skills after upgrade](#refreshing-skills-after-upgrade).

---

## Commands

```bash
quaid version                          # print current binary version (e.g. "quaid 0.22.6")
quaid validate --all --json            # post-upgrade integrity check
quaid skills list --json               # list active skills after upgrade
quaid skills doctor --json             # verify skill resolution and hashes
```

---

## Preferred path — re-run install.sh

The installer is the single source of truth for the asset contract. Re-running
it upgrades in place:

```bash
curl -fsSL https://raw.githubusercontent.com/quaid-app/quaid/main/scripts/install.sh | sh
```

Useful environment overrides (all optional):

| Variable | Default | Effect |
|----------|---------|--------|
| `QUAID_CHANNEL` | `airgapped` | `airgapped` (embedded model, offline) or `online` (downloads model on first use) |
| `QUAID_VERSION` | latest release tag | pin a specific version, e.g. `v0.22.6` |
| `QUAID_INSTALL_DIR` | `~/.local/bin` | where the binary lands |

To upgrade on the same channel you already run, detect it first (see
[Channel detection](#channel-detection)) and pass it through:

```bash
curl -fsSL https://raw.githubusercontent.com/quaid-app/quaid/main/scripts/install.sh \
  | QUAID_CHANNEL=online sh
```

The installer downloads the matching asset, downloads its `.sha256`, verifies
the checksum, and refuses to proceed on mismatch. After it finishes, run
[post-upgrade validation](#post-upgrade-validation).

---

## Channel detection

Two channels ship for every platform:

- **airgapped** — embeds BGE-small; fully offline. The default.
- **online** — downloads/caches the selected model on first semantic use.

Detecting the installed channel is best-effort. Heuristics, in order:

1. If the user told you which channel they run, trust that.
2. `quaid config get model_id` plus `~/.quaid/models/` — an online build that has
   pulled a non-default model is online; a pristine offline DB is typically airgapped.
3. If you cannot tell, default to `airgapped` (the installer default) and tell
   the user which channel you chose so they can override with `QUAID_CHANNEL`.

When in doubt, re-running the installer with no `QUAID_CHANNEL` keeps the
default airgapped channel.

---

## Manual upgrade (fallback)

Only if the installer cannot be used.

### Step 1 — Record current version

```bash
quaid version
```

Output looks like `quaid 0.22.6 (commit abc1234)`. Record it as `current_version`.

### Step 2 — Resolve platform and channel

Build the platform token from the OS and architecture exactly as the installer does:

| OS / arch | Platform token |
|-----------|----------------|
| macOS arm64 (Apple silicon) | `darwin-arm64` |
| macOS x86_64 | `darwin-x86_64` |
| Linux x86_64 | `linux-x86_64` |
| Linux aarch64 | `linux-aarch64` |

The asset name is `quaid-<platform>-<channel>`, where `<channel>` is `airgapped`
or `online`. The checksum asset is the same name with a `.sha256` suffix. These
names are the canonical release contract (`.github/release-assets.txt`). Examples:

| Platform / channel | Asset filename |
|--------------------|----------------|
| Linux x86_64, airgapped | `quaid-linux-x86_64-airgapped` |
| Linux aarch64, online | `quaid-linux-aarch64-online` |
| macOS arm64, airgapped | `quaid-darwin-arm64-airgapped` |
| macOS x86_64, online | `quaid-darwin-x86_64-online` |

### Step 3 — Fetch latest release metadata

```
GET https://api.github.com/repos/quaid-app/quaid/releases/latest
```

Take `tag_name` as `latest_version`. If `latest_version == current_version`,
report "already up to date" and stop. If `latest_version` is older (a
downgrade), stop unless explicitly asked to downgrade.

### Step 4 — Download binary and checksum

```bash
base="https://github.com/quaid-app/quaid/releases/download/<latest_version>"
curl -fL "$base/quaid-<platform>-<channel>" -o quaid.new
curl -fL "$base/quaid-<platform>-<channel>.sha256" -o quaid.new.sha256
```

Do not replace the existing binary yet.

### Step 5 — Verify checksum

```bash
sha256sum -c quaid.new.sha256   # (shasum -a 256 -c on macOS)
```

On failure: delete `quaid.new*`, abort, and never replace the binary.

### Step 6 — Back up and replace

```bash
cp "$(command -v quaid)" "$(command -v quaid).bak"
chmod +x quaid.new
mv quaid.new "$(command -v quaid)"
```

If the install path is not writable, print the exact `sudo` command rather than
escalating unattended:
`sudo cp quaid.new "$(command -v quaid)" && sudo chmod +x "$(command -v quaid)"`.

---

## Post-upgrade validation

```bash
quaid version            # confirm the new version reports correctly
quaid validate --all --json
```

If `validate --all` exits non-zero (violations found):

- Restore the backup if you made one:
  `mv "$(command -v quaid)" "$(command -v quaid).failed" && cp "$(command -v quaid).bak" "$(command -v quaid)"`
- Re-run `quaid version` to confirm the rollback, and report the violations.

If validation passes, print `Upgrade complete. quaid <new_version> is active.`

---

## Refreshing skills after upgrade

Embedded skills travel with the binary, so a successful upgrade already ships
the new defaults. Overrides do not update themselves:

```bash
quaid skills doctor --json
```

Review the output:

- Skills resolving from `embedded://…` are already on the new version.
- Skills resolving from `~/.quaid/skills/…` or `./skills/…` are **shadowing** the
  new embedded copy. If the embedded default changed, your override may be stale.

To adopt the new embedded copy of an overridden skill, either delete the
override (the embedded copy resolves again) or re-materialize it:

```bash
quaid skills extract <name> --force   # overwrite the local override with the new embedded copy
```

`quaid skills doctor` flags shadowing so you can spot overrides that diverged
from a freshly upgraded default.

---

## Failure Modes

| Condition | Behaviour |
|-----------|-----------|
| GitHub API / raw script unreachable | Abort: report the network failure; do not guess a version |
| No matching platform asset | Abort with the resolved platform token in the error |
| Checksum mismatch | Delete downloads, abort. Never replace the binary. |
| Install path not writable | Print the manual `sudo` command; do not escalate unattended |
| `validate --all` fails post-upgrade | Roll back to the `.bak` binary if one exists; report violations |
| Override shadows a changed embedded skill | Warn the user; offer `quaid skills extract <name> --force` |
