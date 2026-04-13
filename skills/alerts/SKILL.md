---
name: gbrain-alerts
description: |
  Interrupt-driven alerts: notify on new contradictions, stale pages, and gap resolution.
---

# Alerts Skill

> Stub — full content to be authored in Phase 3 implementation.

## Alert Types

- New contradiction detected (high priority)
- Knowledge gap resolved
- Page not updated in > 90 days and heavily linked (stale risk)
- Embedding drift detected (re-embed recommended)

## TODO

- [ ] Alert trigger conditions
- [ ] Delivery mechanism (stdout / MCP push / file)
- [ ] Alert suppression and deduplication
- [ ] Priority levels
