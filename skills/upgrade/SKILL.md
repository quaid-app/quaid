---
name: gbrain-upgrade
description: |
  Agent-guided binary and skill updates for gbrain.
---

# Upgrade Skill

> Stub — full content to be authored in Phase 3 implementation.

## Upgrade Flow

1. Check current version: `gbrain version`
2. Fetch latest release metadata from GitHub Releases API
3. Download new binary + SHA-256 checksum
4. Verify checksum before replacing binary
5. Run `gbrain validate --all` after upgrade to confirm DB compatibility
6. Update skills if new skill versions are bundled

## Version Pinning

Skills can declare a minimum binary version. The upgrade skill enforces this.

## TODO

- [ ] Version check against GitHub Releases
- [ ] Checksum verification workflow
- [ ] Rollback on validation failure
- [ ] Skill version negotiation
