---
name: quaid-maintain
description: |
  Maintain brain integrity: detect contradictions, resolve knowledge gaps,
  find orphaned pages, and clean stale assertions.
---

# Maintain Skill

> Stub — full content to be authored in Phase 2 implementation.

## Operations

- `quaid check --all` — run full contradiction detection across all assertions
- `quaid check --resolve <id> --keep <slug>` — resolve a contradiction by keeping one page (the other is superseded by it); omit `--keep` to dismiss without superseding
- `quaid validate --all` — check referential integrity, stale embeddings, broken links
- `quaid gaps` — list unresolved knowledge gaps
- Manual orphan review: pages with no links in or out

## TODO

- [ ] Orphan page detection heuristics
- [ ] Stale assertion cleanup rules
- [ ] Knowledge gap prioritization
