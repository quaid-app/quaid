---
name: gbrain-research
description: |
  Resolve knowledge gaps via web research and targeted ingest.
---

# Research Skill

> Stub — full content to be authored in Phase 3 implementation.

## Workflow

1. Fetch unresolved gaps: `gbrain gaps --limit 10`
2. For each gap: assess sensitivity (internal only by default)
3. If approved for external research: use Exa or web search
4. Ingest findings via the ingest skill
5. Mark gap resolved: `brain_gap_approve` + resolution slug

## Sensitivity Rules

- Gaps are created as `sensitivity = 'internal'`
- Escalation to `external` requires `brain_gap_approve` with user confirmation
- `redacted` sensitivity strips entity names from the query before external use

## TODO

- [ ] Gap prioritization heuristics
- [ ] Research query generation
- [ ] Ingest → gap resolution linkage
- [ ] Sensitivity escalation workflow
