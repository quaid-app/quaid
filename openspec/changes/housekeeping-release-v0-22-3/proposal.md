## Why

This housekeeping batch crosses three public-truth seams that are not covered by any active OpenSpec change:

1. The public roadmap still presents major delivered work as pending even though the shipped surface already includes the knowledge graph layer, daemon runtime, HTTP/SSE transport, and the current 24-tool MCP surface.
2. `origin/main` is ahead of the latest published tag (`v0.22.2`) by one bug-fix commit (`ded7d22`), so the next truthful release lane is `v0.22.3`, not the still-published `v0.22.2` and not the roadmap's stale `v0.23.0` lane.
3. The issue tracker and remote branch list both contain stale operational artifacts that need one evidence-based reconciliation pass before more work piles on top.

Existing active changes (`retrieval-quality-rerank`, `flexible-model-resolution`, `openclaw-skill`) are product-scope lanes. None of them owns roadmap truth repair, release-lane prep for the unreleased main-branch fix, or repo housekeeping.

## What Changes

- Create one operational change lane for the housekeeping/release batch.
- Truth-repair the public roadmap so it matches the shipped codebase and the published-vs-branch release distinction.
- Reconcile open GitHub issues against shipped code and current docs, producing a close/keep/relabel shortlist with evidence.
- Prune remote branches only under a strict ancestry gate: delete branches whose tip is already contained in `origin/main`; preserve any branch still ahead of main until an owner explicitly retires it.
- Prepare the next release lane as `v0.22.3` from `origin/main` commit `ded7d22`, with manifest parity and release-doc truth checked before tagging.

## Impact

- **Docs:** `docs/roadmap_v3.md`, `website/src/content/docs/contributing/roadmap.mdx`, and any release-facing truth copy that still conflates published `v0.22.2` with unreleased `main`.
- **Release lane:** `Cargo.toml`, release branch/tag workflow, and the associated published-install truth pass.
- **Issue tracker:** operational triage only; no runtime behavior changes are implied by this proposal.
- **Git hygiene:** remote branch cleanup under an explicit safety rule.
- **Specs:** No product capability delta is proposed here. This change exists to track release/documentation/housekeeping work coherently.
