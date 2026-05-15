# Leela — housekeeping and v0.22.3 release gate

Date: 2026-05-14T10:44:54.579+00:00
Decision: APPROVE operational batch with gates

## Why

- Public roadmap truth is stale. `docs/roadmap_v3.md` and `website/src/content/docs/contributing/roadmap.mdx` still treat major delivered work as pending even though the shipped surface already includes the knowledge graph layer and the 24-tool MCP/daemon transport stack.
- `origin/main` is one bug-fix commit ahead of the latest published tag: `v0.22.2` points at `e680ebc`, while `origin/main` is `ded7d22` (`fix: generate synthetic corpus when DAB corpus is missing`). The next truthful release lane is `v0.22.3`.
- The issue tracker contains at least one shipped-but-open feature issue (`#135`) and one superseded benchmark snapshot (`#203`).
- The remote branch list has a mix of safely merged branches and still-divergent branches. Cleanup is necessary, but only under a strict ancestry gate.

## Issue shortlist

### Safe to close now

- **#135 — contradiction resolution**
  - Evidence: shipped supersede-chain/runtime surface exists in `src/core/conversation/supersede.rs`, `src/core/conversation/correction.rs`, `tests/fact_resolution.rs`, `tests/supersede_chain.rs`, and `tests/memory_correct.rs`.
  - Evidence: public roadmap already marks contradiction resolution complete in `docs/roadmap_v3.md`.

- **#203 — DAB v0.22.1 benchmark snapshot**
  - Evidence: `#207` supersedes it with the newer `v0.22.2` run. `#203` is a historical report, not an active work item.

### Keep open

- **#134** large corpus performance — still roadmap work; no 10K/50K validation landed.
- **#136** active memory enrichment — still future roadmap work and depends on graph/entity follow-ons.
- **#172, #173, #174** setup/federation/OpenClaw product ideas — no shipped implementation found.
- **#159** outbound cloud redaction layer — sensitivity primitives exist, but no MCP redaction/rehydration path exists.
- **#167** image-to-memory — current spec still says text-only.
- **#75, #76** retrieval quality follow-ons — still active future work.
- **#207** latest benchmark snapshot — keep until its actionable follow-up(s) are split or logged elsewhere.

### Relabel or reframe

- **#197** should be reframed as a docs-truth issue, not a generic enhancement. The repo truth is explicit-assertions-only (`docs/spec.md`, `openspec/specs/structured-assertion-extraction/spec.md`, `src/core/assertions.rs`), while `website/src/content/docs/how-to/contradictions-and-gaps.mdx` still over-claims inferred contradictions.
- **#196** should be reframed before closure. The benchmark evidence in `#203` points to DAB corpus contamination / ranking competition, not yet a proven tokenizer-only bug.
- **#73** should be reframed as “generic long-running admin jobs” if kept open. Quaid already ships SQLite-backed extraction and embedding queues; what is still missing is a user-facing generic submit/status/cancel surface.

## Remote branch cleanup gate

Delete only branches whose tip is already an ancestor of `origin/main`. High-confidence delete candidates from this pass include:

- `origin/copilot/fix-ci-failures-main`
- `origin/copilot/prevent-commits-to-main`
- `origin/docs-overhaul`
- `origin/docs/roadmap`
- `origin/feat/slm-conversation-mem`
- `origin/fix/issue-162-wireup-worker`
- `origin/release/v0.11.0`
- `origin/release/v0.12.0`
- `origin/release/v0.14.0`
- `origin/release/v0.22.0`
- `origin/spec/vault-sync-engine-batch3-v0120`
- `origin/spec/vault-sync-engine-batch4-v0130`
- `origin/spec/vault-sync-engine-batch5-v0140`
- `origin/squad/72-knowledge-graph-layer`

Do **not** auto-delete any branch still ahead of `origin/main` or tied to active/open work. Examples from this pass: `origin/copilot/openspec-housekeeping-20260506`, `origin/fix/mini-bench-queries-dual-corpus`, `origin/agent-data-cleanup`, `origin/openclaw-*` style docs branches, and the remaining old release branches that are not exact ancestors.

## Sequencing / approval gates

### Fry

- Start the release lane from `origin/main` at `ded7d22`.
- Bump `Cargo.toml` to `0.22.3` in the same coherent release-bound commit that lands any release-truth repair touching published-facing files.
- Run `cargo test` before tagging. `release.yml` already hard-fails when `Cargo.toml` and the tag diverge.

### Amy

- Repair roadmap truth before tag push.
- Keep the published-vs-branch distinction explicit: until `v0.22.3` is tagged, public install/download guidance must still name `v0.22.2` as the latest published release.
- Fix the contradiction docs mismatch while already in the docs lane: the how-to page currently implies inferred contradiction extraction that the shipped code does not provide.

### Bender

- Treat remote branch cleanup as destructive repo surgery, not clerical cleanup.
- Delete only merged-into-main branches after one last ancestry check on the exact remote SHA.
- Leave any branch ahead of main for owner review or explicit retirement. No name-based pruning.

---

## Addendum — 2026-05-15T10:21:41.579+00:00

Decision: KEEP the shortlist below as the current issue-tracker truth pass for the checked-out tree; do not claim the close/retitle work landed because GitHub write auth is unavailable in this environment.

### Evidence refresh from current checkout

- `docs/roadmap_v3.md:12-15` and `CHANGELOG.md:7-17` show the repo truth is now `v0.22.3`, with the latest public benchmark gate still called out as `94.4%` on `v0.22.2`.
- `docs/roadmap_v3.md:70-74`, `src/core/supersede.rs`, and `src/core/search.rs:701-703` show shipped supersede chains plus head-only retrieval with an opt-in `include_superseded` escape hatch.
- `src/core/assertions.rs:360-370` and `docs/spec.md:1036-1045` show contradiction detection is intentionally limited to structured zones (`## Assertions` + selected frontmatter), even though lighter-weight docs still summarize it more broadly.
- `src/core/conversation/queue.rs:1-15` and `src/commands/daemon.rs:25-94` show queue/daemon infrastructure already shipped for extraction work, but there is still no generic user-facing async job API.

### Shortlist

#### Safe to close now

- **#135 — feat: contradiction resolution - update/supersede stale facts, not just detect**
  - Reason: shipped supersede-chain semantics already solve the stale-fact/head-truth problem this issue describes.
  - Evidence: `docs/roadmap_v3.md:70-74`, `src/core/supersede.rs`, `src/core/search.rs:701-703`.

- **#203 — DAB v1.0 Results: quaid v0.22.1 — 150/200 🟠 Acceptable**
  - Reason: it is an older benchmark snapshot already superseded by `#207` (`v0.22.2`).
  - Evidence: public GitHub issue search returns `#207` as the newer open DAB results issue; `docs/roadmap_v3.md:15` carries the newer published benchmark truth.

#### Keep open

- **#172** auto-register MCP config — README still documents manual `.mcp.json` setup; no auto-registration command or installer surface is present.
- **#173** git-sync federation — no shipped git-sync/federation workflow exists in CLI/docs beyond ordinary vault usage.
- **#174** OpenClaw plugin manifest — integration docs exist, but no `openclaw.plugin.json` bundle is present in the tree.
- **#159** PII redaction for cloud contexts — sensitivity/gap primitives exist, but there is no outbound MCP redaction layer.
- **#167** image-to-memory — current product/docs remain text-first; no image skill/runtime is present.
- **#134** large-corpus performance — roadmap still lists scale validation as open work (`docs/roadmap_v3.md:190`).
- **#136** active memory enrichment — entity follow-on remains open in roadmap/docs; current shipped graph layer is foundational only.
- **#75** dedup/relevance filtering — retrieval quality remains open work.
- **#76** context compression — no REFRAG-style compression surface exists.

#### Reframe / retitle candidates

- **#197** → `docs: make contradiction detection's structured-assertions-only scope explicit`
  - Why: current code/spec truth is “structured zones only,” so the truthful open delta is docs/runtime expectation alignment, not generic auto-detection magic.
  - Evidence: `src/core/assertions.rs:360-370`, `docs/spec.md:1036-1045`, `docs/getting-started.md:358-377`.

- **#196** → `search: add numeric alias/query normalization for '$75K' vs '75000' retrieval`
  - Why: the repo still lacks numeric aliasing/query normalization, but the current title overstates this as a proven bug rather than a targeted retrieval improvement.
  - Evidence: `src/core/fts.rs:15-76` strips punctuation to spaces but does not normalize `$75K` ↔ `75000`; no changelog entry claims this shipped.

- **#73** → `feat: generic async admin jobs beyond the shipped extraction queue`
  - Why: queue infrastructure already exists; the remaining gap is a generic submit/status/cancel API for long-running non-extraction tasks.
  - Evidence: `src/core/conversation/queue.rs:1-15`, `docs/roadmap_v3.md:70-74`, `src/commands/daemon.rs:25-94`.

### Blocked GitHub writes

`gh auth status` reports: `You are not logged into any GitHub hosts. To log in, run: gh auth login`

Blocked operations:

```bash
gh issue close 135 --repo quaid-app/quaid --comment "Closing as shipped: supersede chains + head-only retrieval now cover the stale-fact resolution described here."
gh issue close 203 --repo quaid-app/quaid --comment "Closing as superseded by newer DAB snapshot #207."
gh issue edit 197 --repo quaid-app/quaid --title "docs: make contradiction detection's structured-assertions-only scope explicit"
gh issue edit 196 --repo quaid-app/quaid --title "search: add numeric alias/query normalization for '$75K' vs '75000' retrieval"
gh issue edit 73 --repo quaid-app/quaid --title "feat: generic async admin jobs beyond the shipped extraction queue"
```
