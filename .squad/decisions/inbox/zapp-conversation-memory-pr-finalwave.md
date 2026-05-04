## Zapp — conversation-memory draft PR final-wave refresh

**Date:** 2026-05-04T07:22:12.881+08:00  
**Requested by:** macro88  
**Change:** conversation-memory-foundations

## Decision

Refresh draft PR #153 so it truthfully says Wave 2 is now approved, then split the remaining product wave in the body: `memory_close_action` is the active in-flight seam, while the file-edit/history-preservation slice stays pre-gated and explicitly unclaimed. Keep the PR draft-only and carry the freshly reproduced OpenSpec conflict count.

## Why

Professor already approved Wave 2 across `b7a0b2d` and `e2fcb65`, so leaving the body at the older "Wave 2 in flight" boundary would now understate shipped progress. But Leela's wave plan still keeps task `10.x` behind Nibbler's pre-gate, so the honest refresh cannot present the whole final wave as landing together; it has to separate the active `memory_close_action` seam from the still-blocked file-edit/history slice while reporting the current six-file spec conflict list against `main`.
