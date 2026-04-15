---
title: Quick Start
description: "Get a brain running in minutes: init, put, search, serve."
---

> **Planned API.** These commands are the target surface for Phase 1 and Phase 2. They reflect the spec but may not be implemented yet.

## 1) Install

Build from source (for now):

```bash
git clone https://github.com/macro88/gigabrain
cd gigabrain
cargo build --release
```

## 2) Initialize a brain

```bash
./target/release/gbrain init ~/brain.db
```

## 3) Write a page

```bash
cat <<'MD' | ./target/release/gbrain put people/alice
---
title: Alice Example
type: person
---

# Alice Example

Above the line: compiled truth.

---

## Timeline

- 2026-04-14 — Met at a demo day; interested in offline-first knowledge systems.
MD
```

## 4) Search it

```bash
./target/release/gbrain search "offline-first"
./target/release/gbrain query "who is interested in offline knowledge systems?"
```

## 5) Start the MCP server

```bash
GBRAIN_DB=~/brain.db ./target/release/gbrain serve
```


---

## What's Next?

- [CLI Reference](/reference/cli/) — full flag and subcommand reference
- [MCP Server](/guides/mcp-server/) — connect Claude Code or any MCP agent
- [Architecture](/reference/architecture/) — how the internals fit together
