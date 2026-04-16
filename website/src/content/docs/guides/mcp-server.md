---
title: MCP Server
description: Run `gbrain serve` and connect any MCP client over stdio JSON-RPC 2.0. Zero config — just a binary and a JSON block.
---

## What `gbrain serve` does

`gbrain serve` starts an MCP server over **stdio**. Your MCP client spawns the
process and talks JSON-RPC 2.0 over stdin/stdout.

## Claude Code config

Add to your MCP client config (example for Claude Code):

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

## Common tool calls (examples)

### Search (FTS5)

```json
{ "query": "river ai", "limit": 10 }
```

### Hybrid query (semantic + keyword)

```json
{ "question": "who has worked with Jensen Huang?", "depth": "auto", "limit": 10 }
```

### Read a page

```json
{ "slug": "people/pedro-franceschi" }
```

### Write a page (optimistic concurrency)

1. `brain_get` to fetch the current `version`
2. `brain_put` with `expected_version`

```json
{
  "slug": "people/pedro-franceschi",
  "content": "# Pedro Franceschi\\n\\nUpdated compiled truth...\\n",
  "expected_version": 3
}
```

## Available tools

### Phase 1 — Core tools

| Tool | Description |
| --- | --- |
| `brain_get` | Read a page by slug |
| `brain_put` | Write/update a page (with optional `expected_version` for optimistic concurrency) |
| `brain_query` | Hybrid FTS5 + vector search; supports `depth: "auto"` for progressive retrieval |
| `brain_search` | FTS5 keyword search |
| `brain_list` | List pages with type/tag filters |

### Phase 2 — Intelligence layer tools

| Tool | Description |
| --- | --- |
| `brain_link` | Create a typed temporal link between two pages |
| `brain_link_close` | Close a temporal link by its database ID |
| `brain_backlinks` | List all inbound backlinks for a page |
| `brain_graph` | N-hop neighborhood graph (nodes + edges JSON) |
| `brain_check` | Run contradiction detection on one page or all pages |
| `brain_timeline` | Show structured timeline entries for a page |
| `brain_tags` | List, add, or remove tags on a page |

### Phase 3 — Data management and brain health tools

| Tool | Description |
| --- | --- |
| `brain_gap` | Log a knowledge gap (query the brain couldn't answer) |
| `brain_gaps` | List unresolved and resolved knowledge gaps |
| `brain_stats` | Brain statistics (page count, link count, contradiction count, db size) |
| `brain_raw` | Store raw structured data (API responses, JSON) for a page |

---

## Phase 3 tool examples

### `brain_gap` — log a knowledge gap

```json
{
  "query": "who funds acme corp?",
  "context": "Asked during company research; no pages found."
}
```

**Returns:** `{ "id": 42, "query_hash": "who-funds-acme" }`

### `brain_gaps` — list knowledge gaps

```json
{ "resolved": false, "limit": 20 }
```

**Returns:** JSON array of gap records with `id`, `query_hash`, `context`, `confidence_score`, `sensitivity`, `resolved_at`, `detected_at`.

### `brain_stats` — brain statistics

```json
{}
```

**Returns:**
```json
{
  "page_count": 1234,
  "link_count": 567,
  "assertion_count": 890,
  "open_gap_count": 12,
  "embedding_count": 3456,
  "active_model": "bge-small-en-v1.5",
  "db_size_bytes": 52428800
}
```

### `brain_raw` — store raw structured data

```json
{
  "slug": "companies/acme",
  "source": "crustdata",
  "data": { "headcount": 320, "funding_total_usd": 45000000 }
}
```

**Returns:** `{ "id": <row_id> }`

---

## Phase 2 tool examples

### `brain_link` — create a temporal link

```json
{
  "from_slug": "people/alice",
  "to_slug": "companies/acme",
  "relationship": "works_at",
  "valid_from": "2023-01-01"
}
```

### `brain_link_close` — close a link

```json
{
  "link_id": 42,
  "valid_until": "2026-01-01"
}
```

### `brain_backlinks` — inbound links to a page

```json
{ "slug": "companies/acme" }
```

**Returns:** JSON array of `{ id, from_slug, relationship, valid_from, valid_until }` objects.

### `brain_graph` — neighborhood graph

```json
{
  "slug": "people/alice",
  "depth": 2,
  "temporal": "active"
}
```

**Returns:**
```json
{
  "nodes": [
    { "slug": "people/alice", "page_type": "person", "summary": "Engineer at Acme." },
    { "slug": "companies/acme", "page_type": "company", "summary": "Enterprise software." }
  ],
  "edges": [
    {
      "from": "people/alice",
      "to": "companies/acme",
      "relationship": "works_at",
      "valid_from": "2023-01-01",
      "valid_until": null
    }
  ]
}
```

Use `temporal: "all"` to include historically closed links.

### `brain_check` — contradiction detection

```json
{ "slug": "people/alice" }
```

Omit `slug` to scan all pages. **Returns:** JSON array of contradiction records:

```json
[
  {
    "page_slug": "people/alice",
    "other_page_slug": "sources/alice-profile",
    "type": "assertion_conflict",
    "description": "Conflicting 'employer' values: 'Acme' vs 'Beta Corp'",
    "detected_at": "2026-04-15T10:00:00Z"
  }
]
```

An empty array means no contradictions found.

### `brain_timeline` — structured timeline entries

```json
{ "slug": "people/alice", "limit": 20 }
```

**Returns:**
```json
{
  "slug": "people/alice",
  "entries": [
    "2026-04-14: Met at demo day [source: meeting/42]",
    "2024-06-01: Joined Acme as staff engineer [source: linkedin-import]"
  ]
}
```

### `brain_tags` — list, add, or remove tags

```json
{ "slug": "people/alice" }
```

Add tags:
```json
{ "slug": "people/alice", "add": ["yc-alum", "key-contact"] }
```

Remove tags:
```json
{ "slug": "people/alice", "remove": ["yc-alum"] }
```

