---
title: Architecture
description: Rust + SQLite brain with FTS5, vectors, and a typed temporal graph.
---

## Tech stack

| Component | Choice |
| --- | --- |
| Language | Rust |
| Database | SQLite via `rusqlite` (bundled) |
| Full-text search | FTS5 (built into SQLite) |
| Vector search | `sqlite-vec` (statically linked) |
| Embeddings | `candle` + BGE-small-en-v1.5 (pure Rust, local) |
| CLI | `clap` |
| MCP server | `rmcp` (stdio JSON-RPC 2.0) |

## Storage model

Everything lives in one SQLite database file:

- `pages`: canonical markdown for each slug
- Side tables for **links**, **timeline entries**, **tags**, **raw data**, and **embeddings**
- WAL mode for durability; `gbrain compact` checkpoints to a single-file artifact for backup/transport

## Hybrid search (keyword + semantic)

GigaBrain is designed for “find what I mean” without sacrificing “exact match”.

- **FTS5**: fast sparse retrieval, great for names, exact phrases, and rare terms
- **Vector search**: dense retrieval for semantic similarity
- **Merge strategy**: planned default is set-union with exact-match short-circuit, so strong lexical hits don’t get buried by semantic noise

For the full design, see the Spec.

