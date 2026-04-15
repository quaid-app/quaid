# Decision: Doc parity fix for Kif's release/coverage review

**Date:** 2026-04-15
**Author:** Fry
**Context:** Kif's task 5.1 review (rejected) identified two doc drift issues blocking release sign-off.

## What changed

1. **spec.md**: All `[owner]` placeholder URLs replaced with `macro88/gigabrain`. Checksum verification updated from `echo ... | shasum --check` to direct `shasum -a 256 --check` against `.sha256` files, matching `release.yml` output format.
2. **install.md**: Coverage artifact description changed from "`lcov.info` artifact" to "`coverage-report` artifact containing `lcov.info` and `coverage-summary.txt`", matching the actual `actions/upload-artifact` name in `ci.yml`.

## Rationale

Kif's review correctly identified that the public doc surface did not match the implemented CI contract. These are the minimum changes to unblock task 5.1 sign-off without scope creep.

## Impact

- Unblocks Kif's 5.1 re-review.
- No workflow, schema, or runtime changes.
