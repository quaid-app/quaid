---
name: "single-writer-bulk-rewrites"
description: "Preserve rename-before-commit invariants by routing bulk file rewrites through the production single-file writer"
domain: "vault-sync"
confidence: "high"
source: "earned"
---

## Context
Use this when a collection-level command needs to rewrite many vault files that already have a correctness-critical single-file writer path.

## Patterns
- Prefer calling the existing single-file writer (`put_from_string`, equivalent mutator, etc.) from bulk admin flows instead of cloning sentinel/tempfile/rename/file_state/raw_import logic.
- Put collection-level gates (serve-owner refusal, writable/restoring/needs_full_sync checks, dry-run counting) around the loop, but let each per-file write go through the hardened writer.
- If bulk rewrites target vault bytes, make the live-owner refusal and the temporary offline lease **root-scoped**, not row-scoped: every collection row sharing the canonical root must block the batch when serve owns any alias, and the offline lease must cover every same-root row for the full rewrite loop.
- Canonicalize legacy aliases during render/write time so migration commands can rewrite stale on-disk keys without widening read paths.

## Examples
- Batch 3 UUID write-back in `src\core\vault_sync.rs::write_quaid_id_to_file` delegates to `src\commands\put.rs::put_from_string`.
- `src\commands\collection.rs::run_uuid_write_back` handles dry-run/live-owner/write-gate checks, then calls the single-file writer per page.

## Anti-Patterns
- Reimplementing a second tempfile/rename/raw_import rotation path for bulk admin commands.
- Counting dry-run candidates with one UUID rule and applying writes with a different one.
