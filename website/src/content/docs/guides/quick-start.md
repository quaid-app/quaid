---
title: Quick Start
description: "Get a brain running in minutes: init, put, search, serve."
---

> **Planned API.** These commands are the target surface for Phase 1 and Phase 2. Phase 1 is in active development — these commands are not yet available. See [Getting Started](/guides/getting-started/) for current status.

## 1) Install

| Method | Status |
| ------ | ------ |
| Build from source | ✅ Available today |
| GitHub Release binary (macOS ARM/x86, Linux x86_64/ARM64) | 🔜 Ships with v0.1.0 |
| `npm install -g gbrain` | ⏳ Deferred — planned follow-on, not in this release |
| One-command curl installer | ⏳ Deferred — planned follow-on, not in this release |

Build from source:

```bash
git clone https://github.com/macro88/gigabrain
cd gigabrain
cargo build --release
```

Once v0.1.0 ships on GitHub Releases, you can download a pre-built binary instead:

```bash
VERSION="v0.1.0"
PLATFORM="darwin-arm64"   # darwin-arm64 | darwin-x86_64 | linux-x86_64 | linux-aarch64
curl -fsSL "https://github.com/macro88/gigabrain/releases/download/${VERSION}/gbrain-${PLATFORM}" -o "gbrain-${PLATFORM}"
curl -fsSL "https://github.com/macro88/gigabrain/releases/download/${VERSION}/gbrain-${PLATFORM}.sha256" -o "gbrain-${PLATFORM}.sha256"
shasum -a 256 --check "gbrain-${PLATFORM}.sha256"
mv "gbrain-${PLATFORM}" /usr/local/bin/gbrain && chmod +x /usr/local/bin/gbrain
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
