# Using Quaid as an OpenClaw Harness

Quaid works as the memory and knowledge layer for agents running on OpenClaw. OpenClaw connects to Quaid over MCP, while Quaid keeps a local SQLite memory database synchronized with one or more markdown collections.

## Prerequisites

- macOS or Linux (required for `quaid serve` watcher runtime)
- `quaid` binary installed
- An initialized memory database
- OpenClaw with MCP server support

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
quaid init ~/memory.db
```

## Attach a vault as a collection

For ongoing sync with Obsidian or any markdown vault:

```bash
quaid collection add notes ~/Documents/notes
```

This performs the initial reconcile and registers the vault for live sync. On a 350-page vault, `collection add` runs in about 5 seconds.

Exclude files using `.quaidignore` at the vault root:

```gitignore
.obsidian/
Templates/
archive/**
```

## Configure OpenClaw

Add Quaid to `openclaw.json` under `mcp.servers`:

```json
{
  "mcp": {
    "servers": {
      "quaid": {
        "command": "quaid",
        "args": ["serve"],
        "env": {
          "QUAID_DB": "/Users/alice/memory.db"
        }
      }
    }
  }
}
```

Set `QUAID_DB` to the absolute path of your `memory.db` file.

## Live sync workflow

1. Initialize: `quaid init ~/memory.db`
2. Attach vault: `quaid collection add <name> <path>`
3. Configure OpenClaw with the MCP block above
4. Start OpenClaw — `quaid serve` starts automatically, launching the MCP server and the live watcher

File edits, creates, and deletes in the vault are reconciled into `memory.db` automatically. No separate sync step is needed while the server is running.

## MCP tools

Quaid exposes 17 MCP tools. OpenClaw agents use these as the durable memory interface.

### Core read/write

| Tool | Description |
| --- | --- |
| `memory_get` | Read a page by slug |
| `memory_put` | Write or update a page (with optional `expected_version` for optimistic concurrency) |
| `memory_query` | Hybrid FTS + vector search; supports `depth: "auto"` for progressive retrieval |
| `memory_search` | FTS keyword search |
| `memory_list` | List pages with type/tag filters |

### Intelligence layer

| Tool | Description |
| --- | --- |
| `memory_link` | Create a typed temporal link between two pages |
| `memory_link_close` | Close a temporal link by its database ID |
| `memory_backlinks` | List all inbound backlinks for a page |
| `memory_graph` | N-hop neighborhood graph (nodes + edges JSON) |
| `memory_check` | Run contradiction detection on one page or all pages |
| `memory_timeline` | Show structured timeline entries for a page |
| `memory_tags` | List, add, or remove tags on a page |

### Data management

| Tool | Description |
| --- | --- |
| `memory_gap` | Log a knowledge gap (query the memory couldn't answer) |
| `memory_gaps` | List unresolved and resolved knowledge gaps |
| `memory_stats` | Memory statistics (page count, link count, db size) |
| `memory_raw` | Store raw structured data (API responses, JSON) for a page |
| `memory_collections` | Read-only collection status: health, state, recovery flags |

## Usage patterns

### `memory_query` vs `memory_search`

Use `memory_query` for:
- Natural-language questions
- Semantic retrieval when wording may not match exactly
- Synthesis across multiple pages

Use `memory_search` for:
- Exact keywords, names, tags, or phrases likely to appear verbatim
- Fast recall when you know the text you want

### When to use `memory_put`

Use `memory_put` when the agent is intentionally creating or updating durable knowledge, not for temporary scratch work.

For updates to an existing page:

1. Call `memory_get` first
2. Read the current `version`
3. Send `memory_put` with `expected_version`

This preserves optimistic concurrency and avoids blind overwrites.

### Collection health checks

`memory_collections` is the right first check when results look stale or writes are blocked:

```json
{}
```

Returns a JSON array of collection status records. Key fields to check:

- `state` — must be `"active"` for the collection to serve queries
- `page_count` — verify expected number of pages
- `last_sync_at` — confirm recent sync
- `embedding_queue_depth` — non-zero means embeddings are pending
- `needs_full_sync` — if `true`, trigger a manual `quaid collection sync <name>`
- `integrity_blocked` — non-null means a reconcile error needs attention

## Recommended operating model

- Use `quaid collection add` for vault-backed knowledge sources
- Use `quaid import` only for one-shot bulk ingest (not ongoing sync)
- Let OpenClaw spawn `quaid serve` — that keeps MCP and watcher lifecycle in one process
- Use `memory_query` first for agent reasoning; fall back to `memory_search` for exact recall
- Use `memory_put` only for durable page updates, not scratch notes
- Poll `memory_collections` during startup or when diagnosing stale results

## Performance reference (350-page vault)

| Operation | Time |
| --- | --- |
| `collection add` | ~5 seconds |
| FTS query (`memory_search`) | ~25ms |
| Semantic query (`memory_query`) | ~95ms |
| All integrity checks | Pass |
