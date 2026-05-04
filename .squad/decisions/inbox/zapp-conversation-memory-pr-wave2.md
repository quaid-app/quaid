## Zapp — conversation-memory draft PR Wave 2 refresh

**Date:** 2026-05-04T07:22:12.881+08:00  
**Requested by:** macro88  
**Change:** conversation-memory-foundations

## Decision

Refresh draft PR #153 so it says Wave 1 is approved and complete, names Wave 2 as the current in-flight scope (`memory_add_turn`, `memory_close_session`, and the first end-to-end conversation integration tests), stays draft, and reports the freshly reproduced OpenSpec conflict list against `main`.

## Why

Professor already approved the Wave 1 artifact, so leaving the draft body framed around the older checkpoint would understate the branch's real progress. A fresh merge simulation now shows six spec-only add/add conflicts rather than the previously listed five, so the truthful update must move both the scope boundary and the conflict count together.
