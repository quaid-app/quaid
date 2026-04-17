---
title: Getting Started
description: Build GigaBrain and create your first brain.db.
---

> GigaBrain is a local-first personal knowledge brain: SQLite + FTS5 + local vector embeddings in one file. `v0.9.1` ships `airgapped` and `online` BGE-small channels.

GigaBrain stores your knowledge as structured pages in a single `brain.db` file. Each page follows the **compiled-truth / timeline** model:

- **Above the line — compiled truth.** Always current. Rewritten when new information arrives. What you know now.
- **Below the line — timeline.** Append-only. Never rewritten. What happened and when.

You search it with full-text keywords and semantic queries. Any MCP-compatible AI agent (Claude Code, etc.) can connect to it over stdio. Everything runs locally — no API keys, no cloud, no Docker.

---

## Status

> **Phase 3 is complete.** `v0.9.1` focuses on dual BGE-small release channels: `airgapped` (embedded weights) and `online` (download/cache on first use).
>
> See the [Roadmap](/contributing/roadmap/) for the full delivery plan and ship gates.

---

## Install options

| Method | Status |
| ------ | ------ |
| Build from source (`cargo build --release`) | ✅ Available now — airgapped default |
| GitHub Release binary (macOS ARM/x86, Linux x86_64/ARM64) | ✅ Available — `v0.9.1` airgapped + online assets |
| `npm install -g gbrain` | 🚧 Staged — online channel by default once published |
| One-command curl installer | ✅ Available — airgapped by default; `GBRAIN_CHANNEL=online` switches channels |

> **BGE-small only.** `v0.9.1` adds exactly two BGE-small release channels and does not introduce a runtime `--model` selector.

---

## Build from source

```bash
git clone https://github.com/macro88/gigabrain
cd gigabrain

# Airgapped channel — default (embeds BGE-small weights into the binary)
cargo build --release
# Binary at: target/release/gbrain

# Online channel — downloads/caches BGE-small on first semantic use
cargo build --release --no-default-features --features bundled,online-model
```

Requirements: Rust toolchain (stable). SQLite and sqlite-vec are bundled. The default build is the **airgapped** channel (embeds BGE-small-en-v1.5 at compile time); use `online-model` for the online variant that downloads/caches BGE-small on first semantic use.

### Cross-compile for static Linux binary

```bash
cargo install cross
cross build --release --target x86_64-unknown-linux-musl      # Linux x86_64 (fully static)
cross build --release --target aarch64-unknown-linux-musl     # Linux ARM64 (fully static)
```

---

## Your first brain

> Phase 1 commands are implemented and available. Build from source to use them now; see [Status](#status) and [Install options](#install-options) above.

### 1. Initialize

```bash
gbrain init ~/brain.db
```

This creates a new `brain.db` file with the full v4 schema — pages, embeddings, links, assertions, and the knowledge-gaps table.

### 2. Import an existing markdown directory

```bash
gbrain import /path/to/notes/ --db ~/brain.db
```

GigaBrain ingests each markdown file, parses frontmatter, splits compiled-truth from timeline, generates embeddings, and writes to the database. Ingest is idempotent — re-running the same file is safe.

### 3. Search

```bash
# Full-text keyword search (FTS5)
gbrain search "machine learning"

# Semantic / hybrid query (FTS5 + vector with set-union merge)
gbrain query "who has worked with Jensen Huang?"
```

### 4. Read and write pages

```bash
# Read a page
gbrain get people/pedro-franceschi

# Write or update a page
cat updated.md | gbrain put people/pedro-franceschi
```

### 5. Work with the knowledge graph

```bash
# Create a typed, temporal link
gbrain link people/pedro-franceschi companies/brex \
  --relationship founded \
  --valid-from 2017-01-01

# Explore graph neighbourhood (2-hop)
gbrain graph people/pedro-franceschi --depth 2

# Close a link that is no longer current
gbrain link people/pedro-franceschi companies/brex \
  --relationship founded \
  --valid-until 2022-12-31
```

### 6. Brain health

```bash
gbrain stats           # page counts, index sizes
gbrain check --all     # contradiction detection
gbrain gaps            # unresolved knowledge gaps
```

### 7. Compact for backup or transport

```bash
gbrain compact         # WAL checkpoint → true single-file brain.db
```

---

## Connect an AI agent via MCP

Add GigaBrain to your MCP client config (e.g., Claude Code's `~/.claude/mcp_config.json`):

```json
{
  "mcpServers": {
    "gbrain": {
      "command": "gbrain",
      "args": ["serve"],
      "env": { "GBRAIN_DB": "/path/to/brain.db" }
    }
  }
}
```

Then start the server:

```bash
gbrain serve
```

The MCP server exposes tools over stdio JSON-RPC 2.0. All 16 tools are active: Phase 1 (`brain_get`, `brain_put`, `brain_query`, `brain_search`, `brain_list`), Phase 2 (`brain_link`, `brain_link_close`, `brain_backlinks`, `brain_graph`, `brain_check`, `brain_timeline`, `brain_tags`), and Phase 3 (`brain_gap`, `brain_gaps`, `brain_stats`, `brain_raw`). See the [MCP Server guide](/guides/mcp-server/) and [Spec](/reference/spec/).

---

## Skills

Skills are markdown files that tell agents _how_ to use GigaBrain. The binary embeds default skills and extracts them to `~/.gbrain/skills/` on first run. Drop a custom `SKILL.md` in your working directory to override any default.

```bash
gbrain skills doctor   # show active skills and resolution order
```

| Skill | Purpose |
| ----- | ------- |
| `skills/ingest/` | Meeting transcripts, articles, documents |
| `skills/query/` | Search and synthesis workflows |
| `skills/maintain/` | Lint, contradiction detection, orphan pages |
| `skills/briefing/` | Daily briefing compilation |
| `skills/research/` | Knowledge gap resolution |
| `skills/enrich/` | External API enrichment |
| `skills/alerts/` | Interrupt-driven notifications |
| `skills/upgrade/` | Agent-guided binary and skill updates |

---

## Page types

`person`, `company`, `deal`, `project`, `concept`, `original`, `source`, `media`, `decision`, `commitment`, `action_item`

The `original` type is for your own thinking — distinct from compiled external intelligence.

---

## Environment variable

| Variable | Default | Purpose |
| -------- | ------- | ------- |
| `GBRAIN_DB` | `./brain.db` | Path to the active brain database |

---

## What's Next?

- [Quick Start](/guides/quick-start/) — five commands to a running brain
- [CLI Reference](/reference/cli/) — full flag and subcommand reference
- [MCP Server](/guides/mcp-server/) — connect Claude Code or any MCP agent
- [Architecture](/reference/architecture/) — how the internals fit together
- [Roadmap](/contributing/roadmap/) — what is built vs. what is coming
