---
title: How It Works
description: Compiled truth + append-only timelines, stored in SQLite.
---

GigaBrain adapts Andrej Karpathy’s compiled knowledge model:

## Above the line: compiled truth

Always current. Rewritten when new information arrives. This is the “what we
believe now” view — optimized for answering questions.

## Below the line: timeline

Append-only. Never rewritten. This is the evidence base — what happened, when,
and where it came from.

## One file, one database

Pages are stored in a single SQLite database (`brain.db`) with:

- **FTS5** for fast keyword search
- **`sqlite-vec`** for vector similarity search
- A **typed, temporal link graph** for “works_at”, “founded”, etc.

The CLI is deliberately thin: the workflows live in markdown `SKILL.md` files
that agents read and follow.

