# Fry — issue #79 / #80 release lane decision

- Date: 2026-04-25
- Scope: release/v0.9.7, issues #79 and #80

## Decision

Use `.github/release-assets.txt` as the single authoritative public release-asset manifest for v0.9.7.

## Why

Issue #79 was not an installer lookup bug in isolation; it was a release-contract drift / partial-release problem amplified by the macOS build break from #80. A checked-in manifest lets the release workflow fail closed on missing assets, keeps installer seam tests honest, and gives docs/reviewer surfaces one exact contract to reference instead of repeating handwritten lists.

## Consequences

- Release verification now reads the manifest instead of maintaining an inline expected array in `release.yml`.
- Release seam/parity tests validate installer/workflow/doc truth against that same manifest.
- Checklist/spec/docs should point to the manifest when naming the public artifact family.
