---
name: gbrain-ingest
description: |
  Ingest meeting notes, articles, documents, and conversations into GigaBrain.
  Handles idempotent ingestion with SHA-256 deduplication.
---

# Ingest Skill

## Overview

The ingest skill processes raw source documents into structured brain pages.
Each file is SHA-256 hashed for idempotent ingestion — re-ingesting the same
file is a no-op unless `--force` is used.

## Commands

### Single file ingest

```bash
gbrain ingest /path/to/document.md
gbrain ingest /path/to/document.md --force  # re-ingest even if hash matches
```

The file must be valid markdown with optional YAML frontmatter. Frontmatter
fields `title`, `type`, `slug`, and `wing` are used if present; otherwise
defaults are derived from the file name and content.

### Batch import

```bash
gbrain import /path/to/directory/          # import all .md files
gbrain import /path/to/directory/ --validate-only  # parse-only, no writes
```

Walks the directory recursively for `.md` files. SHA-256 hashes are checked
against the `import_hashes` table; already-imported files are skipped.
After import, embeddings are automatically refreshed.

### Export

```bash
gbrain export /path/to/output/
```

Exports all pages as canonical markdown files to the output directory.
Files are written to `<output>/<slug>.md`, creating parent directories as needed.

## Idempotency

- SHA-256 of raw file bytes is the idempotency key
- Stored in `import_hashes` table (source_hash, source_path, ingested_at)
- `--force` bypasses the hash check and re-ingests

## Frontmatter Handling

- `slug`: used as-is if present; otherwise derived from file path
- `title`: used as-is if present; otherwise set to slug
- `type`: used as-is if present; defaults to `concept`
- `wing`: used as-is if present; otherwise derived from slug prefix

## Filing Disambiguation

When the same entity could go in multiple wings, prefer the wing
that matches the slug prefix (e.g., `people/` → `people` wing).

