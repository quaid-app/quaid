---
name: gbrain-briefing
description: |
  Generate daily briefings from the brain: what changed, what's new, what shifted.
---

# Briefing Skill

> Stub — full content to be authored in Phase 3 implementation.

## Briefing Structure

1. **What shifted** — pages with `truth_updated_at` or `timeline_updated_at` in the last N days
2. **New pages** — recently created pages
3. **Unresolved contradictions** — `gbrain check --all` summary
4. **Knowledge gaps** — top unresolved gaps by priority
5. **Upcoming** — timeline entries with future dates

## TODO

- [ ] "What shifted" report implementation
- [ ] Prioritization heuristics
- [ ] Briefing format templates
- [ ] Frequency and scope configuration
