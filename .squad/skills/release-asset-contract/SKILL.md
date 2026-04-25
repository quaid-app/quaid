---
name: "release-asset-contract"
description: "Keep installer, workflow, docs, and reviewer checks bound to one release asset manifest."
domain: "release"
confidence: "high"
source: "earned"
---

## Context
Use this when a shell installer, GitHub release workflow, and public docs all reference downloadable binaries. The main failure mode is not just a typo in one place; it is contract drift between independently maintained asset-name tables and partial releases that still look “shipped.”

## Patterns

### Make one manifest authoritative
- Define the public asset schema once: platform set, channel set, binary names, checksum names.
- Derive installer lookup, release verification, and docs/checklist wording from that same contract whenever possible.

### Fail closed on incomplete releases
- A tag is not shippable if any promised asset is missing, even when other platform jobs succeeded.
- Verify exact manifest count as well as required filenames so accidental extras or omissions are both caught.

### Review the public surface, not only the code
- Read installer, workflow, release checklist, and user docs together.
- Stale reviewer checklists are dangerous because they can approve the wrong artifact family even after code was updated.

### Reject installer-only bandages
- Do not accept fallback logic that guesses alternate filenames or silently downgrades channels to hide a missing release asset.
- The correct fix is to repair the release pipeline and keep the installer strict about the published contract.

## Examples

- Dual-channel binaries (`airgapped`, `online`) should publish `gbrain-<platform>-<channel>` everywhere rather than mixing suffixed and unsuffixed names.
- If macOS builds fail and Linux succeeds, the release must remain non-shippable; a nicer 404 message in the installer is not sufficient.

## Anti-Patterns

- Separate handwritten asset lists in installer, workflow, checklist, and docs.
- Manual post-release asset uploads as the primary “fix.”
- Public release notes that imply all platforms shipped before the release workflow actually closed the manifest.
