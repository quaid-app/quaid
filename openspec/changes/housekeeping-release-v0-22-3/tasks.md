## 1. Roadmap truth repair

- [ ] 1.1 Update `docs/roadmap_v3.md` so shipped work is marked as shipped, not pending, and so the release lane distinguishes published `v0.22.2` from branch-prep `v0.22.3`.
- [ ] 1.2 Update `website/src/content/docs/contributing/roadmap.mdx` to match the same shipped-state truth and remaining-work order.
- [ ] 1.3 Audit adjacent public release/status copy (`README.md`, `docs/getting-started.md`, `website/src/content/docs/index.mdx`) and repair only the lines affected by the roadmap/release-lane truth pass.

## 2. Issue reconciliation

- [ ] 2.1 Close issues proven shipped by current code and docs reality (`#135` is the must-close candidate from this pass).
- [ ] 2.2 Close or supersede stale benchmark snapshot issues when a newer snapshot or narrower follow-up already exists (`#203` is the clear superseded candidate from this pass).
- [ ] 2.3 Reframe open issues whose current title/body no longer matches the repo truth (`#197` docs-truth gap, `#196` benchmark/query-normalization framing, `#73` partial queue infrastructure already landed).
- [x] 2.4 Leave future-scope issues open when the codebase still lacks the requested surface (`#172`, `#173`, `#174`, `#159`, `#167`, `#134`, `#136`, `#75`, `#76`).

## 3. Remote branch cleanup

- [ ] 3.1 Produce a delete list containing only remote branches whose tip SHA is already an ancestor of `origin/main`.
- [ ] 3.2 Exclude branches that are tied to active OpenSpec work, the current release lane, or any branch that is still ahead of `origin/main`.
- [ ] 3.3 Delete the approved merged-only branch set and record what was removed.

## 4. Release v0.22.3

- [ ] 4.1 Branch from `origin/main` at `ded7d22`, not from an older release lane or stale side branch.
- [ ] 4.2 Bump the release manifest surface to `0.22.3` in one coherent release-bound commit.
- [ ] 4.3 Run the repo validation gate (`cargo test`) before tagging.
- [ ] 4.4 Tag and publish `v0.22.3` only after the roadmap/release truth pass lands.
