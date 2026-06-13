---
name: quaid-setup
description: |
  Bootstrap a fresh Quaid install in one guided flow: initialize the database,
  attach a vault collection, install the background serve daemon, and wire MCP
  clients (Claude Code, Cursor). Use when onboarding a new machine or a new
  agent that has never run Quaid before.
min_binary_version: "0.22.0"
---

# Setup Skill

## Overview

This skill orchestrates the first-run bootstrap sequence so agents and operators
do not have to hand-roll it each time. The four steps are:

1. **Initialize** the memory database (`quaid init`).
2. **Attach** a vault collection so notes sync into the brain (`quaid collection add`).
3. **Install** the background daemon so the index stays fresh (`quaid daemon install`).
4. **Wire** MCP clients so Claude Code / Cursor can call the `memory_*` tools
   (`quaid setup --register-mcp`).

All steps are local-first and non-destructive. The MCP-wiring step never
rewrites a config wholesale — it parses, merges, and backs up — so it is safe to
re-run.

---

## Operating constraints

- **Local-first paths.** The default DB lives at `~/.quaid/memory.db` and the
  default vault at `~/.quaid/vault`. Honor `QUAID_DB` / `--db` if the operator
  has set one; do not silently relocate an existing store.
- **Collection naming.** Use a short, lowercase, hyphen-free collection name
  (e.g. `notes`, `journal`). The collection name is a logical label, not the
  path.
- **Daemon ownership.** Only one daemon should own a given database. Run
  `quaid status` first; if a daemon is already installed and running, do not
  install a second one — report it and stop.
- **Idempotency.** Every step is safe to re-run. `init` on an existing DB is a
  no-op; `setup --register-mcp` reports "already up to date" when nothing
  changes.

---

## Commands

### 1. Initialize the database

```bash
quaid init                      # creates ~/.quaid/memory.db
quaid init /custom/path.db      # or an explicit path
```

`init` is non-interactive and uses the current default model channel. It honors
`QUAID_MODEL` / `--model` and `QUAID_DB` / `--db`. Running it on an existing
database leaves the file untouched.

### 2. Attach a vault collection

```bash
quaid collection add notes ~/Documents/notes
```

`collection add` performs the initial scan and attaches the directory as a
live-sync collection. Add `--read-only` for sources you never want Quaid to
write back into.

### 3. Install the background daemon

```bash
quaid status                    # check whether a daemon already owns the DB
quaid daemon install            # install + start the platform-native service
quaid daemon status             # confirm it is running
```

The daemon keeps the index fresh as the vault changes. Service management
(launchd on macOS, systemd on Linux) is handled by `daemon install`; see the
daemon-install docs for log locations and uninstall steps.

### 4. Wire MCP clients

```bash
quaid setup --register-mcp           # merge into ~/.claude/mcp.json + ~/.cursor/mcp.json
quaid setup --register-mcp --dry-run # preview the diff without writing
```

`--register-mcp` parses each client config, merges
`mcpServers.quaid = {command:"quaid", args:["serve"], env:{QUAID_DB:<resolved path>}}`
while preserving any servers already configured, and writes atomically with a
`.bak` of the previous file. It prints exactly what changed. Restart the client
after wiring so it picks up the new server.

---

## Operator handoff

After running the flow, report back exactly what was created so the operator can
review and finish any manual steps:

- **Database:** path, and whether it was newly created or already existed.
- **Collection:** name, root path, and read-only vs. writable.
- **Daemon:** installed/started, or skipped because one already owned the DB.
- **MCP clients:** which config files were created or updated, where the `.bak`
  files landed, and a reminder to **restart the MCP client** so it loads the new
  `quaid` server.

Flag anything that still needs manual attention (e.g. a client whose config was
malformed JSON and therefore skipped, or a daemon that requires elevated
privileges to install).
