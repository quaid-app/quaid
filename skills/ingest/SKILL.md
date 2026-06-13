---
name: quaid-ingest
description: |
  Ingest meeting notes, articles, documents, and conversations into Quaid.
  Handles idempotent ingestion for exact-byte duplicates and vault-backed sync.
---

# Ingest Skill

## Overview

The ingest skill processes raw source documents into structured brain pages.
Re-ingesting the same exact bytes is a no-op unless `--force` is used.

## Commands

### Single file ingest

```bash
quaid ingest /path/to/document.md
quaid ingest /path/to/document.md --force  # re-ingest even if hash matches
```

The file must be valid markdown with optional YAML frontmatter. Frontmatter
fields `title`, `type`, `slug`, and `wing` are used if present; otherwise
defaults are derived from the file name and content.

### Vault-backed batch ingest

```bash
quaid collection add notes /path/to/directory/
quaid serve
```

`quaid collection add` performs the initial scan and attaches the directory as a
live-sync collection. `quaid serve` keeps the index fresh on Unix platforms.

### Export

```bash
quaid export /path/to/output/
```

Exports all pages as canonical markdown files to the output directory.
Files are written to `<output>/<slug>.md`, creating parent directories as needed.

## Idempotency

- Exact raw file bytes are the idempotency key for duplicate single-file ingest
- Active source bytes and paths are stored in `raw_imports`
- `--force` bypasses the hash check and re-ingests

## Frontmatter Handling

- `slug`: used as-is if present; otherwise derived from file path
- `title`: used as-is if present; otherwise set to slug
- `type`: used as-is if present; defaults to `concept`
- `wing`: used as-is if present; otherwise derived from slug prefix

## Filing Disambiguation

When the same entity could go in multiple wings, prefer the wing
that matches the slug prefix (e.g., `people/` → `people` wing).

## Conversation capture (MCP)

Beyond batch documents, Quaid can ingest a live conversation turn-by-turn and
extract structured facts from it in the background. This path is MCP-only — there
is no CLI subcommand for it — and is the discovery workflow QMD-style exporters
cannot do.

- `memory_add_turn` — append one turn (`role`: `user` / `assistant`, `content`,
  optional `timestamp`, optional `metadata`) to an open `session_id`. When
  extraction is enabled in config, each turn schedules a debounced extraction job;
  the response includes `extraction_scheduled_at`.
- `memory_close_session` — mark the session closed and trigger a final extraction
  pass over the accumulated turns.

Typical flow:

```
1. Pick a stable session_id for the conversation (e.g. the client's thread id).
2. For each message, call memory_add_turn(session_id, role, content).
3. When the conversation ends, call memory_close_session(session_id).
4. Extraction runs in the daemon: decisions, preferences, facts, and action
   items become extracted pages under <namespace>/extracted/<type>/<slug>.
```

Extraction policy (dedup vs. supersede vs. coexist) is not a skill decision — it
is hard-coded in `src/core/conversation/` and runs unattended. The skill's job is
only to feed clean turns and close the session.

## Namespaces

Both turns and pages can live in a **namespace** — an isolated partition of the
brain (e.g. a project, a client, or a privacy boundary).

- `memory_add_turn` and `memory_close_session` accept an optional `namespace`;
  turns and their extracted pages are scoped to it.
- `quaid put <slug> --namespace <ns>` and `quaid ingest` honour the same scoping.
- Omitting the namespace writes to global memory.

Keep a conversation's turns in one namespace so its extracted facts file
alongside the rest of that namespace's knowledge. Manage namespaces with
`quaid namespace`.

