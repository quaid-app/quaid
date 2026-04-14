---
title: MCP Server
description: Run `gbrain serve` and connect any MCP client over stdio JSON-RPC.
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

From the planned surface area:

`brain_query`, `brain_search`, `brain_get`, `brain_put`, `brain_ingest`,
`brain_link`, `brain_link_close`, `brain_backlinks`, `brain_graph`,
`brain_timeline`, `brain_tags`, `brain_list`, `brain_check`, `brain_gap`,
`brain_gaps`, `brain_stats`, `brain_raw`.

