# Using Quaid as an OpenClaw Harness

Quaid works as the memory and knowledge layer for agents running on OpenClaw. OpenClaw connects to Quaid over MCP, while Quaid keeps a local SQLite memory database synchronized with one or more markdown collections.

## Prerequisites

- macOS or Linux
- `quaid` v0.9.9 or later
- An OpenClaw install that supports MCP server configuration

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/quaid-app/quaid/main/scripts/install.sh | sh
```

Verify:

```bash
quaid --version
```

## Initialize

```bash
quaid init ~/.quaid/memory.db
```

## Attach a vault as a collection

For ongoing sync with Obsidian or any markdown vault:

```bash
quaid collection add notes ~/Documents/notes
```

This creates the collection, performs the initial reconcile, and registers a watcher for live sync. On a 350-page vault, `collection add` takes about 5 seconds.

Generate embeddings after indexing:

```bash
quaid embed --db ~/.quaid/memory.db
```

Use `.quaidignore` at the vault root to exclude files or patterns:

```gitignore
.obsidian/
Templates/
archive/**
```

## Configure OpenClaw

Add the Quaid block to `openclaw.json` under `mcp.servers`:

```json
{
  "mcp": {
    "servers": {
      "quaid": {
        "command": "quaid",
        "args": ["serve"],
        "env": {
          "QUAID_DB": "/Users/alice/.quaid/memory.db"
        }
      }
    }
  }
}
```

`QUAID_DB` must point to the memory database that OpenClaw agents should use.

To test the server outside OpenClaw:

```bash
QUAID_DB=~/.quaid/memory.db quaid serve
```

## Live sync workflow

1. `quaid init ~/.quaid/memory.db`
2. `quaid collection add <name> <path>` — attach one or more vaults
3. `quaid embed` — generate embeddings (required for `memory_query`)
4. Configure OpenClaw to spawn `quaid serve`
5. Start OpenClaw

`quaid serve` owns both the MCP server and the file watcher. Edits, creates, and deletes in the vault are picked up automatically.

## MCP tools

The latest public release (`v0.18.0`) exposes 22 MCP tools via `quaid serve`. This branch expands that surface to 24 tools while adding the unreleased `v0.19.0` conversation-memory extraction + correction follow-on. All tool names use the `memory_*` prefix.

### `memory_query` vs `memory_search`

Use `memory_query` for:

- Natural-language questions
- Synthesis across multiple pages
- Semantic retrieval when wording may not match exactly

Use `memory_search` for:

- Exact keywords, names, titles, or tags
- Fast recall when you know the text you want

### `memory_put`

Use `memory_put` when the agent is intentionally creating or updating durable knowledge. For updates to an existing page:

1. Call `memory_get` first to read the current `version`
2. Send `memory_put` with `expected_version` to preserve optimistic concurrency

### `memory_collections` — health check

`memory_collections` returns the status of all attached collections. Check these fields when results look stale or writes are blocked:

| Field | Healthy value |
|-------|--------------|
| `state` | `"active"` |
| `embedding_queue_depth` | `0` |
| `needs_full_sync` | `false` |
| `recovery_in_progress` | `false` |
| `integrity_blocked` | `null` |

Example response:

```json
[
  {
    "name": "notes",
    "root_path": "/Users/alice/Documents/notes",
    "state": "active",
    "writable": true,
    "page_count": 350,
    "last_sync_at": "2026-04-27T03:12:04Z",
    "embedding_queue_depth": 0,
    "needs_full_sync": false,
    "recovery_in_progress": false,
    "integrity_blocked": null
  }
]
```

## Benchmark reference

Verified against [quaid-evals](https://github.com/quaid-app/quaid-evals) on MSMARCO dev corpus, Quaid v0.9.9:

| Test | Result |
|------|--------|
| DAB total | 193 / 215 (90%) |
| FTS (`memory_search`) | 40 / 40 — perfect |
| Semantic (`memory_query`) | 48 / 50 |
| Performance | 30 / 30 — perfect |
| MCP | 20 / 20 — perfect |
| `collection add` | ~5s / 350 pages |
| FTS latency | ~25ms |
| Semantic latency | ~95ms |

Live results: [benchmark.quaid.app](https://benchmark.quaid.app)

## Recommended operating model

- Use `quaid collection add` for vault-backed knowledge sources
- Use `quaid ingest` for one-off markdown files outside a live collection
- Let OpenClaw spawn `quaid serve` — keeps MCP and watcher in one process
- Run `memory_query` first for agent reasoning; fall back to `memory_search` for exact recall
- Use `memory_put` only for intentional durable page updates
- Poll `memory_collections` on startup or during incident diagnosis

## Migration from GigaBrain / pre-v0.9.9

Existing databases (schema v5) are incompatible with v0.9.9 (schema v6). See [MIGRATION.md](../MIGRATION.md) for the full migration guide.

Quick summary:

```bash
# Export using the pre-v0.9.9 binary command shown in MIGRATION.md
# (kept there so legacy command details stay in one place)

# Initialize new database
quaid init ~/.quaid/memory.db

# Re-ingest the exported markdown
quaid collection add migrated ~/brain-backup
```
