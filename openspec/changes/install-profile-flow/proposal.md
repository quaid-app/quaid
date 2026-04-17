---
id: install-profile-flow
title: "Install: auto-write PATH and GBRAIN_DB to shell profile"
status: proposed
type: enhancement
owner: fry
reviewers: [leela]
created: 2026-04-19
depends_on: simplified-install
closes: ["#36", "#41"]
---

# Install: auto-write PATH and GBRAIN_DB to shell profile

## Why

After a successful install, `scripts/install.sh` prints PATH and GBRAIN_DB hints but makes no
changes to the user's shell profile. For human users this is easy to miss; for agent users
(OpenClaw, CI) it is a silent failure — the binary installs correctly but is unreachable after
the session ends because `~/.local/bin` is not in `$PATH` and `GBRAIN_DB` is never set.

Issue #36 (filed by beta tester doug-aillm) named this as an adoption blocker for agent users.
Issue #41 is a duplicate that adds a second requirement: document a two-step install for
sandboxed environments where piping a remote script directly to `sh` is blocked.

## What Changes

### 1. `scripts/install.sh` — automatic profile write

After a successful install, the script:

1. Detects the user's preferred shell profile (`~/.zshrc` for zsh, `~/.bashrc` for bash,
   `~/.profile` as fallback), based on `$SHELL` and file existence.
2. Appends the `PATH` export line if `$INSTALL_DIR` is not already present in the profile.
3. Appends the `GBRAIN_DB` export line if it is not already present in the profile.
4. Prints a confirmation: `Added PATH and GBRAIN_DB to ~/.zshrc. Run: source ~/.zshrc`
5. Respects a `--no-profile` flag (or `GBRAIN_NO_PROFILE=1` env var) to skip all profile
   writes entirely.

Idempotency: the script checks for the presence of each line before appending, so
repeated installs do not duplicate entries.

### 2. `scripts/install.sh` — two-step install documentation block

After the success message, the script prints a two-step alternative for sandboxed environments:

```
For sandboxed environments (agents, restricted shells):
  curl -fsSL https://raw.githubusercontent.com/macro88/gigabrain/main/scripts/install.sh \
    -o gbrain-install.sh && sh gbrain-install.sh
```

### 3. `packages/gbrain-npm/scripts/postinstall.js` — matching GBRAIN_DB tip

The npm postinstall already prints a GBRAIN_DB tip. Update the message to match the new
shell installer wording for consistency.

### 4. Docs — README Quick Start and install guide

- README.md Quick Start: add a note that the installer writes to the shell profile automatically
  and document `--no-profile` / `GBRAIN_NO_PROFILE` opt-out.
- `website/src/content/docs/guides/install.md`: add a "Sandboxed / agent environments" section
  with the two-step install pattern and the `GBRAIN_NO_PROFILE=1` env var.
- `docs/getting-started.md`: update install walkthrough to reflect automatic profile setup.

## Non-Goals

- Windows support (separate proposal when Windows is a supported target).
- Fish shell support — fish uses a different config format. Document as unsupported; fall through
  to `~/.profile` for fish users.
- Modifying any profile other than the one detected — no multi-profile writes.

## Impact

- `scripts/install.sh`: new `write_profile` and `detect_profile` shell functions; new
  `--no-profile` flag parsing; updated post-install output.
- `packages/gbrain-npm/scripts/postinstall.js`: minor wording update to GBRAIN_DB tip.
- `README.md`, `website/src/content/docs/guides/install.md`, `docs/getting-started.md`:
  documentation updates.
