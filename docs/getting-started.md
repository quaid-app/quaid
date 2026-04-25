# Getting Started with GigaBrain

> GigaBrain is a local-first personal knowledge brain: SQLite + FTS5 + local vector embeddings in one file. `v0.9.7` keeps the current vault-sync release surface and adds the macOS release-build fix plus a canonical release-asset contract.

## What it does

GigaBrain stores your knowledge as structured pages in a single `brain.db` file. Each page follows the **compiled-truth / timeline** model:

- **Above the line — compiled truth.** Always current. Rewritten when new information arrives. What you know now.
- **Below the line — timeline.** Append-only. Never rewritten. What happened and when.

You search it with full-text keywords and semantic queries. Any MCP-compatible AI agent (Claude Code, etc.) can connect to it over stdio. Everything runs locally — no API keys, no cloud, no Docker.

---

## Status

> **Phase 3 is complete, and the first vault-sync slice is shipped.** The current release is `v0.9.7`: it preserves the current vault-sync surface, fixes the macOS release build, and keeps `airgapped` + `online` assets on one canonical `gbrain-<platform>-<channel>` contract.
>
> See [roadmap.md](roadmap.md) for the full delivery plan.

---

## Install options

| Method | Status |
| ------ | ------ |
| Build from source (`cargo build --release`) | ✅ Available now — airgapped default |
| GitHub Release binary (macOS ARM/x86, Linux x86_64/ARM64) | ✅ Available — `v0.9.7` airgapped + online assets |
| `npm install -g gbrain` | 🚧 Staged — online channel by default once published |
| One-command curl installer | ✅ Available — airgapped by default; set `GBRAIN_CHANNEL=online` for online |

> **Configurable BGE models.** The `online` build selects `small` (default), `base`, `large`, or `m3` via `GBRAIN_MODEL` / `--model`. The `airgapped` build embeds BGE-small-en-v1.5.

---

## Build from source

```bash
git clone https://github.com/macro88/gigabrain
cd gigabrain
cargo build --release
# Binary at: target/release/gbrain (airgapped channel — default; embeds BGE-small-en-v1.5)

# Online channel (downloads/caches BGE-small on first semantic use)
cargo build --release --no-default-features --features bundled,online-model
```

Requirements: Rust toolchain (stable). SQLite and sqlite-vec are bundled. The default build is the **airgapped** channel (embeds BGE-small-en-v1.5 at compile time); the explicit `online-model` build produces the online variant that downloads/caches BGE-small on first semantic use.

### Cross-compile for static Linux binary

```bash
cargo install cross
cross build --release --target x86_64-unknown-linux-musl      # Linux x86_64 (fully static)
cross build --release --target aarch64-unknown-linux-musl     # Linux ARM64 (fully static)
```

---

## Your first brain

> **Phase 1 commands** are implemented. **Phase 2 commands** (graph, check, gaps) are implemented. **Phase 3 commands** (validate, call, pipe, skills) are implemented. Build from source to use all features now; see [Status](#status) and [Install options](#install-options) above.

> **Post-install note:** The shell installer (`scripts/install.sh`) automatically adds `PATH` and `GBRAIN_DB` to your shell profile. If you built from source or used the manual GitHub Releases download, add these to your profile yourself:
> ```bash
> export PATH="$HOME/.local/bin:$PATH"
> export GBRAIN_DB="$HOME/brain.db"
> ```
> For CI/agent environments that manage PATH externally, skip profile writes with `GBRAIN_NO_PROFILE=1`.

### 1. Initialize

```bash
gbrain init ~/brain.db
```

This creates a new `brain.db` file with the full v5 schema — pages, embeddings, links, assertions, knowledge-gaps table, and (in `v0.9.6`) collections, file_state, and raw_imports tables.

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

> **Unix only in `v0.9.6`.** `gbrain serve` now owns the vault-sync runtime, so macOS/Linux are the supported hosts for MCP on this release line. On Windows it returns `UnsupportedPlatformError`; use the portable CLI commands (`search`, `query`, `get`, `list`, etc.) there until a safe non-Unix serve contract lands.

The MCP server exposes tools over stdio JSON-RPC 2.0.

**Phase 1 tools (core read/write):** `brain_get`, `brain_put`, `brain_query`, `brain_search`, `brain_list`

**Phase 2 tools (intelligence layer):** `brain_link`, `brain_link_close`, `brain_backlinks`, `brain_graph`, `brain_check`, `brain_timeline`, `brain_tags`

**Phase 3 tools (gaps, stats, raw data):** `brain_gap`, `brain_gaps`, `brain_stats`, `brain_raw`

**vault-sync tools:** `brain_collections` — read-only per-collection status including state, ignore diagnostics, and recovery flags

All 17 tools are live in the current `v0.9.6` release. See [spec.md](spec.md#mcp-server) for tool signatures.

---

## Skills

Skills are markdown files that tell agents _how_ to use GigaBrain. The binary embeds default skills and extracts them to `~/.gbrain/skills/` on first run. Drop a custom `SKILL.md` in your working directory to override any default.

All 8 skills are production-ready as of Phase 3.

```bash
gbrain skills list     # show all active skills with source paths
gbrain skills doctor   # verify skill hashes and detect shadowing
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

`person`, `company`, `deal`, `project`, `concept`, `original`, `source`, `media`, `decision`, `commitment`, `action_item`, `area`, `resource`, `archive`, `journal`

The `original` type is for your own thinking — distinct from compiled external intelligence.

### Page types and PARA folder structure

When you run `gbrain import`, page types are resolved in three tiers:

1. **Frontmatter `type:` field** — if your markdown file includes `type: project` (or any non-blank type) in the YAML frontmatter, that value is used. Frontmatter always wins. Blank (`type: `) or null (`type: null`) values are treated as absent and fall through to tier 2.
2. **Top-level folder inference** — if no usable `type:` field is present, GigaBrain infers the type from the first folder in the relative path. This supports the PARA method and common Obsidian vault layouts:

| Folder name | Inferred type |
|-------------|---------------|
| `Projects` (or `1. Projects`) | `project` |
| `Areas` (or `2. Areas`) | `area` |
| `Resources` (or `3. Resources`) | `resource` |
| `Archives` (or `4. Archives`) | `archive` |
| `Journal` / `Journals` | `journal` |
| `People` | `person` |
| `Companies` / `Orgs` | `company` |

Folder matching is **case-insensitive** and strips leading numeric prefixes (e.g. `1. `, `02. `) that Obsidian users commonly use for sort order.

If the folder doesn't match any known name, or if the file is at vault root with no sub-folder, the page defaults to `concept`.

**Example:** importing `2. Areas/Health/exercise.md` (with no `type:` in frontmatter) yields a page with type `area`.

To override inference, add `type: <your_type>` to the file's YAML frontmatter.

---

## Environment variable

| Variable | Default | Purpose |
| -------- | ------- | ------- |
| `GBRAIN_DB` | `./brain.db` | Path to the active brain database |
| `GBRAIN_NO_PROFILE` | `0` | Set to `1` to skip shell profile writes during install |

---

---

## Phase 2: Intelligence layer

> Phase 2 commands are fully implemented. Build from source to use them.

### Graph traversal

Walk the knowledge graph from any page, up to N hops out:

```bash
# 2-hop neighbourhood, active links only (default)
gbrain graph people/alice --depth 2

# 3-hop including expired links
gbrain graph people/alice --depth 3 --temporal all

# JSON output for programmatic use
gbrain graph people/alice --depth 2 --json
```

Example output:
```
people/alice
  → companies/acme (works_at)
    → people/bob (colleague)
  → projects/atlas (leads)
```

### Contradiction detection

Check one page or every page in the brain for conflicting assertions:

```bash
# Check a single page
gbrain check --slug people/alice

# Check the entire brain
gbrain check --all

# JSON output
gbrain check --all --json
```

Example output:
```
[people/alice] ↔ [sources/linkedin-2023]: alice employer assertion conflicts: Acme vs. Beta Corp
1 contradiction(s) found across 2 pages.
```

### Knowledge gaps

GigaBrain automatically records low-confidence queries as knowledge gaps. List and triage them:

```bash
# List unresolved gaps (default)
gbrain gaps

# Include resolved gaps
gbrain gaps --resolved

# Limit output
gbrain gaps --limit 10

# JSON output for scripting
gbrain gaps --json
```

Example output:
```
[1] who-founded-acme (confidence: 0.21, unresolved)
[2] atlas-project-status (confidence: 0.18, unresolved)
2 gap(s) found.
```

Use the `skills/research/` skill to resolve gaps: the research workflow queries external sources, writes new pages, and calls `brain_gap` to mark each gap resolved.

---

## Phase 3: Validation, scripting, and skills

> Phase 3 commands are implemented. Build from source to use them.

### Database integrity validation

Check the brain for broken links, duplicate assertions, or stale embeddings:

```bash
# Run all integrity checks
gbrain validate --all

# Targeted checks
gbrain validate --links        # referential integrity and interval overlaps
gbrain validate --assertions   # assertion dedup and supersession chains
gbrain validate --embeddings   # embedding model consistency

# JSON output for scripting
gbrain validate --all --json
```

Example output:
```
[links] OK — 14 links checked, 0 violations
[assertions] 1 violation: duplicate assertion subject=people/alice predicate=employer
[embeddings] OK — 312 chunks checked, active model consistent
1 violation(s) found.
```
Exit 0 means clean; exit 1 means violations were found.

### Raw MCP tool invocation

Call any MCP tool directly from the CLI without starting the server:

```bash
gbrain call brain_stats '{}'
gbrain call brain_get '{"slug": "people/alice"}'
gbrain call brain_gap '{"query": "who founded acme corp"}'
```

### JSONL pipeline mode

Stream tool calls via stdin, one JSON object per line:

```bash
echo '{"tool":"brain_search","input":{"query":"machine learning"}}' | gbrain pipe
cat queries.jsonl | gbrain pipe > results.jsonl
```

### Skill inspection

```bash
gbrain skills list     # list active skills with source resolution path
gbrain skills doctor   # verify SHA-256 hashes, detect override shadowing
```

---

## vault-sync-engine: Collections and live-sync

> These commands are shipped in `v0.9.6`. The current release line exposes the landed vault-sync slice while keeping Windows `serve`/restore hosting and IPC follow-ons explicitly deferred.

### Attach a vault

```bash
# Attach a directory as a named collection
gbrain collection add work /path/to/my-obsidian-vault

# List all collections
gbrain collection list

# Show extended status: ignore errors, quarantine count, recovery flags
gbrain collection info work
```

Once attached, `gbrain serve` starts a file watcher for the collection. Changes you make in Obsidian or any editor are debounced over 1.5 s and flushed via the stat-diff reconciler.

> **Unix only.** `gbrain serve` and all live-watcher functionality require a Unix platform (macOS or Linux). On Windows, `gbrain serve` returns `UnsupportedPlatformError` because the full serve/runtime surface is intentionally gated in this release line; portable CLI reads/searches still work there, but MCP hosting and vault-byte write-through remain deferred.

### Ignore patterns

```bash
# Add a glob to .gbrainignore (dry-run validates before writing)
gbrain collection ignore add work "drafts/**"

# List active patterns
gbrain collection ignore list work

# Remove a pattern
gbrain collection ignore remove work "drafts/**"

# Clear all user patterns (built-in defaults remain)
gbrain collection ignore clear work --confirm
```

### Collection-aware slugs

When multiple collections are active, prefix slugs with the collection name:

```bash
gbrain get "work::people/alice"
gbrain put "work::people/alice" < page.md
```

`brain_search`, `brain_query`, and `brain_list` accept an optional `collection` filter; they default to the only active collection when exactly one exists.

### Quarantine

Pages that are deleted or renamed while holding DB-only state (links, assertions, knowledge gaps, contradictions, or `raw_data`) are quarantined rather than hard-deleted.

```bash
gbrain collection quarantine list work
gbrain collection quarantine export work --out quarantine.json
gbrain collection quarantine discard work <page-slug>
```

> **Unix only.** `gbrain collection quarantine restore` is implemented and available on Unix (macOS/Linux). On Windows it returns `UnsupportedPlatformError`. Restore moves a quarantined page back to active status, refuses occupied targets, and requires the destination parent directories to already exist:
>
> ```bash
> gbrain collection quarantine restore work <page-slug> <relative-path>
> ```
>
> The IPC/online-handshake path (automatic restore on watcher reconnect) and Windows restore host support remain deferred and are not yet wired.

Auto-sweep TTL (`GBRAIN_QUARANTINE_TTL_DAYS`, default 30) silently discards clean quarantined pages only — pages with DB-only state are never auto-discarded.

---

## Next steps

- Read [roadmap.md](roadmap.md) to understand what is built vs. what is coming.
- Read [contributing.md](contributing.md) to start contributing.
- Read [spec.md](spec.md) for the full technical specification.
