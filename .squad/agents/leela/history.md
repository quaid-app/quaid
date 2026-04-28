- When a spec has both BDD scenarios and task description summaries, the BDD scenario is the governing contract. Task descriptions are prose summaries that can drift. Always resolve conflicts by reading the scenario block directly.
- A "thin harness, fat skills" SKILL.md needs exactly four elements to be agent-executable: (1) exact command sequences, (2) configurable parameters table, (3) failure modes table, and (4) explicit statements on what the skill does NOT do automatically. All five Phase 3 skills contain all four.
- Approval workflow dependencies (like `brain_gap_approve`) that are not yet binary commands must be explicitly documented as such in the skill — without that note, an agent will try to shell-exec them and fail silently.

---

## 2026-04-16 Phase 3 Skills Review Complete — Task 8.3

**Session:** leela-phase3-skills-review (176s, claude-sonnet-4.6)  
**Timestamp:** 2026-04-16T06:02:45Z

**What happened:**
- Task 8.3 APPROVED: All five Phase 3 SKILL.md files pass completeness, clarity, and agent-executability review.
- Stale threshold: **30 days (per spec scenario line 28, not 90 days).**
- Task 1.2 corrected in `tasks.md` from >90 days to >30 days.
- Decision merged to `decisions.md`. Orchestration log written.

**Phase 3 progression:** Unblocked. Can proceed to cross-checks (8.1, 8.2, 8.4–8.7) and implementation (Groups 2–7).


---

## 2026-04-16 Phase 3 Task 8.1 — Core Fixes Retry (leela-phase3-core-fixes-retry)

**Session:** leela-phase3-core-fixes-retry (866s, gpt-5.2-codex)  
**Timestamp:** 2026-04-16T07:20:47Z

**What happened:**
- Task 8.1 REVISION SUBMITTED: Addressed Professor Phase 3 core review blockers.
  - Decision D-L1: Skills resolution now truly embedded via `include_str!()` with `embedded://skills/<name>/SKILL.md` labeling. Layers `~/.gbrain/skills` and `./skills` overrides in order, removing cwd dependency.
  - Decision D-L2: `gbrain validate --embeddings` treats unsafe `embedding_models.vec_table` values as validation violations and skips dynamic SQL in that case, preventing unsafe queries while still surfacing the problem.
- 2 decisions merged to `decisions.md`.
- Orchestration log written.
- **Status:** Task 8.1 left for re-review by different revision author per phase 3 workflow (Leela under reviewer lockout).

**Next:** Await Nibbler re-review before proceeding to core-lane cross-checks.

---

## 2026-04-17 Phase 3 Archive Pass — Leela Sync

**Session:** leela-phase3-archive  
**Timestamp:** 2026-04-17

**What happened:**
- Audited three OpenSpec changes: `p3-skills-benchmarks`, `p3-polish-benchmarks`, `sprint-0-repo-scaffold`.
- Found two actual regressions that tasks.md had marked complete but were not:
  1. `ci.yml` missing `benchmarks` job (task 7.1 note was aspirational — added the job for real)
  2. `cargo clippy` failing with 2 violations in `tests/concurrency_stress.rs` (task 8.6 was wrong — fixed both)
- Removed a false pre-existing archive: `openspec/changes/archive/2026-04-17-p3-skills-benchmarks/` had `status: complete` but 8.2 and 8.4 open. Removed. Active copy now source of truth.
- Archived `p3-polish-benchmarks` (all tasks genuinely complete) → `openspec/changes/archive/2026-04-17-p3-polish-benchmarks/`.
- Cleaned up `sprint-0-repo-scaffold` active copy (archive from 2026-04-15 was already present).
- Left `p3-skills-benchmarks` active: 8.2 Nibbler MCP adversarial review and 8.4 Scruffy benchmark reproducibility check are genuinely open.
- Updated README.md and website roadmap from "✅ Complete" to honest "🔄 Implementation complete — reviewer sign-off pending."
- Updated `now.md` to reflect current team focus: Nibbler and Scruffy reviewer gates.
- Created `openspec/changes/p3-skills-benchmarks/` and `p3-polish-benchmarks/` artifact files on disk (they only existed as input artifacts, not in the filesystem).

**Decisions filed:** `.squad/decisions/inbox/leela-phase3-archive.md`

## Learnings

- **Tasks.md notes can be forward-looking lies.** When a task note says "✓ Added X", always verify X exists in the codebase before accepting it. Optimistic notes written by a previous session are not the same as completed work.
- **Archiving with open gates is an honesty violation.** A pre-existing archive had `status: complete` but two open reviewer checkboxes. The archive process must check the actual task status, not just copy files. Removed the false archive.
- **OpenSpec artifact files may not exist on disk even when listed as input artifacts.** The input artifact system passes file content as context; the actual filesystem files may be absent. Always check with PowerShell before trying to edit.
- **False archive removal is the right call when reviewer gates are genuinely open.** The team gate system (Nibbler adversarial review, Scruffy reproducibility verification) has real engineering value. Archiving before those gates close removes accountability and prevents the review from happening.

## Learnings — Phase 3 Final Reconcile (2026-04-17)

- **Inbox decisions confirm gate closure; tasks.md must reflect it.** Nibbler and Scruffy filed inbox decisions that closed their gates. The tasks.md still had `[ ] 8.2` — inbox decisions don't self-propagate into task checklists. Always update tasks.md to reflect closed gates before archiving.
- **Archive/active split is a binary state.** The correct resolution for "active copy untracked + archive deleted" is: update active tasks, restore archive from HEAD, overwrite with updated files, delete active. There is never a valid "both exist" state.
- **PR body must be the last thing updated, not the first.** It reflects the final state of the branch. Updating docs, archiving, and committing first ensures the PR body accurately describes what is actually in the branch.
- **The `.squad/decisions/inbox/` is gitignored by design.** Decision records there are local-only scratchpads; they don't need to be committed. This is correct — they serve the team's working session, not the permanent repo record.
- **`git restore <dir>` correctly restores all deleted tracked files under that path.** Useful for recovering a previously-archived set of files that were deleted in the working tree.

## 2026-04-18 Focus File Update — simplified-install / v0.9.0

**What was done:**
- Updated `.squad/identity/now.md` to replace stale "Phase 3 complete — v1.0.0 ready to tag" posture with truthful `simplified-install` / `v0.9.0` shell-first rollout status.
- Old branch reference (`phase3/p3-skills-benchmarks`) replaced with active branch (`simplified-install`).
- Status summary now distinguishes: fully done (A, B, C, D.1, D.3, D.4) vs. environment-blocked (D.2, D.5).

**Key facts about the simplified-install change:**
- Phase A (shell installer) and Phase B (npm scaffolding) are complete. No blocking implementation gaps.
- D.2 (npm postinstall live test) is blocked: Windows host hits EBADPLATFORM; WSL has no Node runtime; v0.9.0 is not a real GitHub Release yet.
- D.5 (publish-npm.yml token guard) is static-review only; no local Actions runner; `npm publish --dry-run` blocked by existing `gbrain@1.3.1` on public registry.
- npm public publication stays gated behind: (1) confirmed shell-installer test on real v0.9.0 release, (2) NPM_TOKEN secret configured in repo.

**Key file paths:**
- Proposal: `openspec/changes/simplified-install/proposal.md`
- Tasks: `openspec/changes/simplified-install/tasks.md`
- Shell installer: `scripts/install.sh`
- npm package: `packages/gbrain-npm/`
- Publish workflow: `.github/workflows/publish-npm.yml`
- Focus file: `.squad/identity/now.md`

**Learning:**
- Focus files go stale across phase transitions. Update `now.md` at the start of each new change, not just at the end of the previous one. A stale focus file misleads every agent that reads it on spawn.

## 2026-04-19: Dual Release v0.9.1 OpenSpec Unblock

**Scope:** Cleanup and validation prep for `bge-small-dual-release-channels` change.

**Work:**
- Removed stale `dual-release-distribution/` duplicate change tree (old "slim" naming, unapproved)
- Populated `bge-small-dual-release-channels/tasks.md` with 10 machine-parsable tasks (A–D)
- Validated A.1–C.3 tasks are correctly marked done via code inspection
- Confirmed product naming lock: `airgapped` and `online` only

**Learning:**
- Empty tasks.md on an OpenSpec change should be surfaced as a blocker during proposal validation, not discovered during cleanup. The tooling should catch this.
- Duplicate changes with conflicting naming conventions should be explicitly archived or deleted, not left to create hazard for future implementation references.

---

## 2026-04-19: Beta Feedback Triage — Three OpenSpec Lanes

**Session:** leela-beta-openspec  
**Branch:** squad/beta-feedback-openspec

**What happened:**
- Triaged four GitHub issues (#36, #38, #40, #41) from beta tester doug-aillm.
- Determined #41 is a duplicate of #36 (same root cause; #41 adds the two-step sandbox install note only).
- Created three separate OpenSpec lanes — separate is correct because ownership, risk level, and code areas are distinct.

**Lanes created:**

| Lane | Closes | Owner | Risk |
|------|--------|-------|------|
| `install-profile-flow` | #36, #41 | fry | Low — shell scripting + docs only |
| `assertion-extraction-tightening` | #38 | professor | High — changes runtime behavior for all vaults; Nibbler review gated |
| `import-type-inference` | #40 | fry | Low — 1-function change in migrate.rs + docs |

**Key file paths:**
- `openspec/changes/install-profile-flow/` — proposal, design, tasks
- `openspec/changes/assertion-extraction-tightening/` — proposal, design, tasks
- `openspec/changes/import-type-inference/` — proposal, design, tasks
- `.squad/decisions/inbox/leela-beta-openspec.md` — routing decision

## Learnings

- **Issue #41 as duplicate pattern:** When two issues share a root cause, the right call is to close one as duplicate and capture the additive notes (two-step sandbox install) in the surviving lane's tasks, not create two proposals.
- **Assertion false positives are a trust problem, not just a bug:** The fix scope should be narrow (scoped extraction) to avoid introducing new false negatives. The structural choice — `## Assertions` section as opt-in contract — makes the behavior teachable and predictable.
- **PARA is the dominant Obsidian structure:** Any import tooling that ignores top-level folder names will fail this user base on first run. The folder-to-type mapping is a first-class feature, not a nice-to-have.
- **High-risk core changes should be explicitly gated on Nibbler adversarial review in the proposal.** Putting Nibbler in `reviewers:` in the frontmatter is insufficient; the tasks.md should have an explicit Nibbler phase gate (Phase D.1 pattern from p3-skills-benchmarks).

## Vault Sync Foundation Repair -- 2025-07-18

## Vault Sync Batch C Gate — 2026-04-22

**Session:** Leela gate review of Batch C (tasks 2.4a/2.4b/2.4c/2.4d/4.2/4.3/4.4/5.2).

**Verdict: REJECT**

**What was solid:**
- `rustix` dependency: correctly platform-gated in `[target.'cfg(unix)'.dependencies]`.
- `fs_safety.rs`: all six fd-relative primitives correct — O_NOFOLLOW, O_DIRECTORY, AT_SYMLINK_NOFOLLOW semantics sound. Windows stubs return `UnsupportedPlatformError` (not success-shaped).
- `stat_file_fd`: correctly wraps `fs_safety::stat_at_nofollow`. Direct tests cover success + nofollow on Unix.
- `has_db_only_state`: returns `Err` (not `Ok(false)`) — the critical safety fix from Batch B.
- `stat_diff`, `full_hash_reconcile`, `reconcile`: honest stubs, correctly scoped, tests pin the contracts.
- `tasks.md`: truthful about what's deferred.

**Blocking finding — Gate Rule 1 (Overstatement):**
`reconciler.rs` references `fs_safety::open_root_fd` and `OwnedFd` inside `#[cfg(unix)]` blocks with no corresponding conditional imports. On Windows (where CI runs), `#[cfg(unix)]` blocks are skipped entirely, so tests and clippy both pass. On Linux/macOS, these are hard compile errors. Task 5.2 claims "Foundation complete: Unix path uses `fs_safety::open_root_fd` for bounded walk root" — but code that doesn't compile on Unix is not foundation-complete on Unix.

**Secondary findings (doc errors, not individually blocking):**
- `stat_file` doc says "Prefers fd-relative fstatat when both parent_fd and name are provided" — but the signature is `fn stat_file(path: &Path)`. No `parent_fd` parameter exists. Doc describes a hypothetical API.
- `stat_file_fallback` says "uses lstat (follows symlinks)" — `lstat` does NOT follow symlinks. The function uses `fs::metadata()` equivalent to `stat()`. Comment has the two syscalls confused.

**Fix path (Fry, targeted, sub-30-min):**
1. Add `#[cfg(unix)] use crate::core::fs_safety;` to `reconciler.rs`
2. Add `#[cfg(unix)] use rustix::fd::OwnedFd;` to `reconciler.rs`
3. Bundle doc fixes in `file_state.rs` in the same pass

**Decision artifact:** `.squad/decisions/inbox/leela-vault-sync-batch-c-gate.md`

What happened: Professor rejected Fry's foundation slice. 181 tests were failing. Fry locked out; Leela owned the repair pass.

Root causes found:
1. pages.collection_id NOT NULL -- v5 schema added this FK but no write helper supplied it. Every INSERT failed on NOT NULL constraint.
2. pages.uuid NOT NULL -- UUID lifecycle (tasks 5a.*) is not yet wired; making the column non-nullable was premature.
3. ingest_log removed from schema -- the spec removes it in the reconciler slice but ingest.rs, embed.rs, and migrate.rs still depend on it.
4. ON CONFLICT(slug) -- the v5 unique constraint changed to UNIQUE(collection_id, slug); SQLite requires the ON CONFLICT target to match. All upsert paths broke.
5. search_vec missing quarantine filter -- FTS was correct (triggers) but vector search joined pages without filtering quarantined rows.

Fixes applied:
- schema.sql: collection_id DEFAULT 1, uuid DEFAULT NULL with partial unique index, ingest_log re-added as compatibility shim.
- db.rs: ensure_default_collection() -- INSERT OR IGNORE of collection id=1 name=default on every open_connection().
- ingest.rs and migrate.rs: ON CONFLICT(collection_id, slug).
- inference.rs: AND p.quarantined_at IS NULL added to search_vec.
- tasks.md: checkboxes and notes corrected to match actual state.

Result: cargo test -- 0 failures (was 181).

Key lessons:
- A schema change adding NOT NULL FKs is never complete until ALL write paths supply the column. Use DEFAULT or nullable when lifecycle is incomplete.
- Removing a table that existing code depends on is two-step: (1) replace dependents, (2) drop table. Never in one step.
- Marking tasks as done when downstream insert sites are incomplete is an integrity violation. Reviewers treat checkboxes as verified guarantees.

## 2026-04-22 Vault Sync Batch B Narrow Repair

**Session:** 20260422-191436 (Leela narrow repair pass)

**What happened:**
- Professor gated Batch B on two safety-critical issues: has_db_only_state() returning Ok(false) (success-shaped default on a delete-protecting predicate) and module header claiming reconciler "replaces" import_dir when migrate::import_dir() is still live.
- Leela applied focused repair: changed has_db_only_state() to return explicit Err("not yet implemented..."), updated module header to "WILL replace" with clarifying timeline, updated task 5.1 completion note for consistency.

**Three decisions recorded:**
1. **D1: has_db_only_state Error Semantics** — Safety-critical stub returns Err, forcing explicit error handling instead of silent "safe to delete" assumption
2. **D2: Module Documentation Accuracy** — "Will replace" + timeline vs. "replaces"; clarifies which path is live
3. **D3: Task 5.1 Truthfulness** — Completion note separates "file created" (✅ complete) from "replace logic wired" (Batch C/task 5.5)

**Outcome:**
- cargo test: 0 failures (442 lib + 40 integration, both channels)
- Batch B gate now clean
- Decisions merged to canonical ledger
- Inbox cleared; orchestration and session logs written

**Key learning:** Safety-critical predicates must not have success-shaped defaults when unimplemented. Explicit error failure mode is self-documenting and prevents accidental wiring before completion. This reinforces Rust best practices: deferred work must be loudly failed, not silently safe.

## 2026-04-22 Vault Sync Batch C — Repair Pass (Approved)

**Session:** Leela repair owner after Batch C initial rejection by Professor and Scruffy on missing Unix imports and overclaimed task completion.

**What happened:**
- Initial gate feedback identified two blockers: missing #[cfg(unix)] use declarations for s_safety and OwnedFd in 
econciler.rs, and overclaimed task completion (2.4c, 4.4, 5.2 checked when only scaffolding existed).
- Leela made four targeted fixes: added conditional imports, demoted tasks from complete to pending, removed non-existent parent_fd parameter from stat_file doc, fixed lstat vs stat semantics in stat_file_fallback doc.
- No new functionality, no feature expansion — pure rectification of overstatement.

**Key decisions:**
1. **Safety-critical stubs fail explicitly:** 
econcile() and ull_hash_reconcile() return Err("not yet implemented") until real walk/hash/apply logic lands. Rationale: stubs on recovery paths cannot return Ok(empty stats) — that silently grants "reconciliation ran successfully" when no reconciliation actually happened.
2. **Conditional imports required:** #[cfg(unix)] use declarations are syntactically required at module scope for Unix-gated function signatures that reference Unix-only types. Windows CI skips these blocks silently; missing imports cause hard compile errors on Linux/macOS.
3. **Task demotion for honesty:** Tasks 2.4c (walk semantics), 4.4 (full_hash_reconcile), 5.2 (reconcile phase structure) downgraded from [x] to [ ] because only scaffolding types/signatures exist, not the described behavior.
4. **Doc fixes bundled:** Platform-split function docs must describe actual implementation, not hypothetical future versions. Fixed two isolated doc errors in ile_state.rs.

**Validation:** All 439 lib tests pass. No regressions. Ready for re-gate.

**Outcome:** Scruffy and Professor both approved after validation. Foundation seams locked with direct tests. Ready to land as explicitly unwired base for Batch D.


### 2026-04-22 17:02:27 - Vault-Sync Batch E Repair

**Session:** Narrow repair pass after Nibbler's adversarial rejection

**Problem identified:**

Hash-rename guard in src/core/reconciler.rs used whole-file size for ≥64-byte threshold, allowing template notes with large frontmatter and tiny body to incorrectly satisfy the check and inherit the wrong page_id.

**Repair (narrow scope):**

Modified only MissingPageIdentity, NewTreeIdentity, load_missing_page_identities, load_new_tree_identities, and hash_refusal_reason:

1. Replace size_bytes fields with body_size_bytes computed from trimmed parsed content
2. MissingPageIdentity.body_size_bytes = compiled_truth.trim().len() + timeline.trim().len()
3. NewTreeIdentity.body_size_bytes = body.trim().len() (post-frontmatter)
4. hash_refusal_reason() now checks body_size_bytes < 64, not whole-file size
5. Refusal strings renamed: missing_below_min_bytes → missing_below_min_body_bytes

**Tests added:**

One regression test: template_note_with_large_frontmatter_and_tiny_body_is_never_hash_paired
- Proves large-frontmatter note cannot satisfy 64-byte body threshold
- Verifies quarantine classification (not hash_renamed pairing)

**Validation:**

- No new structs, functions, or Batch E scope expansion
- Surrounding docs/comments remain honest about deferred work
- cargo test --quiet: all 439 tests pass
- cargo clippy: clean

**Gate outcome:**

Nibbler's prior blocker resolved. Batch E is landable.

**Rule for future implementers:**

The 64-byte threshold in content-hash identity guards ALWAYS refers to body content after frontmatter. Whole-file size is NOT a proxy. Consistent with spec tasks 5.8a0 and 5.8e which explicitly say 'body size ≤ 64 bytes after frontmatter'.

## Learnings

- **Legacy mutators must share the newest safety gate:** If a restore/full-sync interlock is added only to modern collection-aware paths, old compatibility commands (`ingest`, `import`) become silent reopen holes. The safe pattern is to put the gate in the shared low-level writer entrypoints or apply a global fail-closed check before any legacy write transaction starts.
- **Offline success must stop before reopen authority:** For restore/remap flows split between CLI and runtime recovery, offline commands may reach Tx-B / pending-full-sync, but they must not also perform attach completion. Leave `needs_full_sync=1` in place and force RCRT to reacquire ownership before writes reopen.
- **Restore integrity proofs must post-date the implementation holes they test (2026-04-23):** When two reviewers propose different orderings for the same pair of clusters, the tiebreaker is whether the proof tests exercise honest code. If the underlying implementation has a known live hole (e.g., offline `begin_restore` not persisting `restore_command_id`), writing tests against it produces false confidence. The right sequence is: fix the code first (even one batch earlier), then write the proofs. A test that "passes" against broken code certifies the wrong invariant and poisons the proof record.
- **Batch K rescope — Professor's ordering prevails over Nibbler's (2026-04-23):** When ordering two non-overlapping clusters, and one cluster (restore proofs) has a live code gap that would make its tests vacuous, that cluster must come SECOND so the code fix can land in the first cluster's peer batch. Nibbler's valid adversarial concerns (identity theft, manifest tamper, Tx-B residue) are preserved by targeting them at K2, not by running them against broken K1 code. The shared write gate (`CollectionReadOnlyError` via `9.2b`/`17.5qq11`) must also precede restore tests because those tests exercise write-blocking during restore state.
- **K1 truth-gap repair — vault-byte gate vs write interlock are separate concerns (2026-04-23):** When Scruffy's proof lane found `1.1b`, `9.2b`, and `17.5qq11` not fully provable, the narrowest fix was: (1) for `1.1b`, add MCP-response tests confirming `page_id` is returned in the brain_gap response for both slug and slug-less paths; (2) for `9.2b`/`17.5qq11`, add an explicit scope note to tasks.md and an MCP-path test for `brain_put` confirming the `CollectionReadOnly` gate applies ONLY to vault-byte mutators (`brain_put`/`gbrain put`), not to DB-only mutators (`brain_gap`, `brain_link`, `brain_check`, `brain_raw`). Professor's ruling drove this: vault-byte gate and DB-only write interlock are separate concerns with separate check functions (`ensure_collection_vault_write_allowed` vs `ensure_collection_write_allowed`). Nibbler's nuance — slug-bound `brain_gap` must still take the write interlock — was already correct in the code; the repair surfaced it via tests. Rule: when a gate is added to a subset of mutators, always document explicitly which mutators are IN scope and which are OUT, and add tests for both.
- **Online-model build is broken on Windows MSVC via stack overflow in rustc (2026-04-23):** The `GBRAIN_FORCE_HASH_SHIM=1 cargo test --quiet --no-default-features --features bundled,online-model` command fails on Windows x86_64-pc-windows-msvc with `STATUS_STACK_BUFFER_OVERRUN` (0xc0000409) during compilation of the `tokenizers` crate. This is a pre-existing platform limitation — rustc's type inference overflows its default stack when processing tokenizers' complex generics on Windows. This is unrelated to any K1 code changes; the default-features test suite (`cargo test --quiet`) passes fully.
- **K2 truth-gap repair — deferred notes must be re-evaluated when the implementation advances (2026-04-23):** Scruffy kept 17.11 deferred citing "attach completion still depends on serve/RCRT." This was accurate at Batch I when `complete_attach` did not exist in the CLI path. By K2, `finalize_pending_restore_via_cli` chains `finalize_pending_restore` (Tx-B) → `complete_attach` (runs `full_hash_reconcile_authorized` + sets `state='active'`) entirely within the CLI process — no serve/RCRT required. The end-to-end proof (`offline_restore_can_complete_via_explicit_cli_finalize_path` in `tests/collection_cli_truth.rs`, `#[cfg(unix)]`) calls the real binary and confirms `state=active`, `needs_full_sync=0`, and correct `root_path` after `--finalize-pending`. Rule: deferred notes contain a premise. When the implementation changes in a way that invalidates the premise, the deferred note becomes a false claim and must be superseded — not left in place. The proof reviewer's job at gate time is to re-check every deferred note against the current code, not to carry forward a conclusion from an earlier state.
- **Batch L rescope — recovery-authority coherence beats infrastructure-vs-proof grouping (2026-04-23):** When two reviewers split a batch along different lines (Professor: split by infrastructure vs. proof tier; Nibbler: split by recovery authority), the right tiebreaker is recovery-authority coherence, not layer grouping. Grouping by layer (infrastructure in one batch, proofs in the next) leaves the proof batch mixing two independent failure modes under a single pass/fail surface. Grouping by recovery authority ensures each batch answers exactly one question — "did this recovery path work?" — with its own proof boundary. Rule: when a multi-infrastructure batch is split, ask which split line produces the narrowest, most falsifiable proof claim per batch, not which split line minimizes code duplication. Also: a shared initialization task (`11.1`) that serves two different recovery paths should be split into sub-tasks along the recovery-authority line before implementation starts, to prevent accidental scope drift during implementation.

- **Post-N1 next-slice selection — MCP-proven library seam drives CLI parity batch (2026-04-24):** When a batch (N1) closes a slug-routing seam on MCP surfaces only, the correct follow-on is CLI parity for that same seam — not new MCP tooling or filter parameters. The key diagnostic is: which CLI commands bypass `resolve_slug_for_op` and go directly to bare-slug DB queries? (e.g. `graph.rs` calls `neighborhood_graph` with `WHERE slug = ?1`, skipping collection-aware resolution entirely.) 13.3 exists to close exactly that gap. 13.5 and 13.6 add new capabilities (collection filter params, new MCP tool) that require fresh design decisions and are correctly deferred. Rule: after an MCP-only seam closure, the next batch is the CLI-parity mirror, not the next MCP capability layer.

- **Post-13.3 next-slice selection — `brain_collections` (13.6) before collection-filter semantics (13.5) (2026-04-24):** After CLI parity closes (13.3), the two remaining section-13 tasks are 13.5 (optional collection filter on search/query/list with write-target default logic) and 13.6 (new `brain_collections` read-only MCP tool). 13.6 is the right first pick: it is a pure read-only tool addition with no mutation path, no default-filter semantics, no write-target logic, and a single clear proof test (17.5ddd). 13.5's "write-target in single-writer setups, all collections otherwise" default encodes state logic that `brain_collections` will expose — landing 13.5 before 13.6 means the 13.5 default is under-tested and potentially overclaims. The slug-resolution proof cluster (17.5ss–17.5vv6) is a separate scope that should be scoped as its own slice after 13.6, not co-mingled. Rule: when two tasks in the same group compete, prefer the one that establishes observable ground truth (13.6) over the one that filters on it (13.5).

## Post-13.6 next-slice selection (2026-04-24)

NEXT SLICE: **13.5** — optional `collection` filter on `brain_search`, `brain_query`, `brain_list`

13.6 is now closed, which was the one prerequisite blocking 13.5: the "write-target in single-writer setups" default arm needed `brain_collections` to expose observable collection state before the default could be tested honestly. That ground truth now exists. 13.5 is the only remaining section-13 functional task: three read-only MCP tool handlers each gain one optional `collection` parameter. It touches zero mutation paths, zero write-gates, zero filesystem interaction, and zero startup/watcher/IPC/dedup surfaces. The proof tests are narrow and falsifiable: (1) explicit `collection` name filters correctly to that collection; (2) unknown `collection` name returns a stable error; (3) absent param in a single-collection brain returns results from that collection (trivially equivalent to all-collections); (4) absent param in a multi-collection brain returns results from the write-target collection only. All four proofs are purely DB-read operations with no ordering dependency on open infrastructure.

DEFER: `17.5aa5` (stable-absence `ignore_parse_errors` expansion — requires `17.5aa`/`aa2-4` ignore-CLI infrastructure first), `17.5ss–17.5vv6` slug-resolution proof cluster (Write routing proofs for WriteCreate/WriteUpdate/WriteAdmin, own slice after 13.5), `17.5aa–aa4` ignore-file CLI commands, watcher/IPC/remap/dedup/startup surfaces, any mutator coverage, post-landing coverage/docs/release/cleanup agenda.

**Trap — "single-writer setup" ambiguity:** The 13.5 default arm ("filter by write-target in single-writer setups, all collections otherwise") contains a spec seam analogous to the 13.6 `ignore_parse_errors` stable-absence arm. The `is_write_target=1` invariant guarantees exactly one write-target collection at all times, so "single-writer setup" cannot mean "exactly one write-target exists" — that is always true. The tasks.md note must pin the exact predicate before Fry starts: the only stable reading is `COUNT(*) FROM collections WHERE state != 'detached' = 1` (only one active collection in the brain, making the filter moot) vs. `> 1` (multiple active collections, where the write-target is the relevance-scoped default). If this predicate is left undefined, a reviewer will dispute whether "all collections" fires in any real multi-collection scenario, producing a gate dispute identical to the 13.6 schema seam. The fix is a single explicit note in the 13.5 tasks.md entry before implementation starts. Additionally, the "all collections" arm in the multi-collection case is NOT a wider-search default — it means "search ALL collections when no filter is passed and there is no write-target to default to"; since a write-target always exists by invariant, the "all collections" arm is the single-collection-brain case only. This must be stated explicitly or the implementation will invert the two arms.

## 2026-04-25 Quarantine Third Revision

**Session:** Leela single-author repair after re-review rejection of Fry+Mom second revision.

**Learnings:**

- **Two-gate distinction:** ensure_collection_write_allowed (state/needs_full_sync check) vs ensure_collection_vault_write_allowed (state check + collection.writable=0 check) are separate gates for separate concerns. A DB-write gate is not a vault-byte gate. Any function that writes bytes to the collection filesystem must call ensure_collection_vault_write_allowed, not ensure_collection_write_allowed. Review every restore/remap/ingest entry point for this gate - a wrong call is invisible until you try to write to a read-only collection.

- **Post-rename residue pattern:** After renameat(temp to target) succeeds, the target file is live on disk. Any subsequent failure in the same error path (fsync, stat, hash, DB upsert, tx.commit) leaves a disk residue. The correct mitigation is: in every error arm after the rename, call unlinkat(parent_fd, target_name) best-effort before returning. This is distinct from temp-file cleanup (which uses the temp path and applies before the rename). If a file is at its final path, the error arm owns its cleanup.

- **Post-rename residue is code-only testable:** Triggering a post-rename failure in integration (without mocking) requires causing fsync, stat, hash, or SQLite to fail after a successful renameat. Not feasible in standard integration tests. A structural code fix plus a tasks.md note is the correct disposition. Do not defer -- the code fix IS the closure.

- **now.md edit tool limitation:** The edit tool fails on lines with trailing whitespace differences. Use PowerShell: (Get-Content -Raw) -replace then Set-Content -NoNewline.

- **Post-quarantine-batch sequencing (2026-04-25):** After Bender's quarantine truth repair backed out `quarantine restore`, the next truthful stop-point is a narrow quarantine-restore fix (two Nibbler blockers only: parent-fsync after unlink in the restore path + no-replace install semantics at renameat time). Coverage backfill (`17.17c`, `17.17d`, `17.5rr`, `17.5ss–vv6`, `17.4`) and section-16 docs are independent and run in parallel — they should NOT block or wait on the restore slice. The decision: quarantine seam takes Fry + Professor pre-gate + Nibbler review; coverage takes Scruffy + Professor + Nibbler; docs can go to any available agent. Do NOT open watcher mutation handlers, IPC, embedding queue, or UUID write-back until quarantine restore is fully closed and coverage is done. Key invariant: named invariant tests (`17.17c` raw_imports_active_singular, `17.17d` quarantine_db_state_predicate_complete) cover already-live data-loss surfaces and should never queue behind an implementation slice.
- **Restore re-enable slice narrowing and contract (2026-04-25):** The quarantine restore seam has two mandatory blockers before re-enable is safe: (1) post-unlink cleanup must fsync the parent directory after every unlink, and (2) install-time target absence checks must use no-replace semantics (renameat must fail if target exists at install time, not just at pre-check time). A concurrent-creation race that clobbers after pre-check but before install is a data loss surface. The gate is hard: both must be provable in code + tests before restore reopens.

### 2026-04-25: Issues #79 + #80 combined fix — v0.9.7 release routing

**What:** Routed issues #79 (install.sh 404 on darwin-x86_64-airgapped) and #80 (fs_safety.rs macOS compile error) as a single release lane. Both issues had a shared root cause: all macOS builds failed in v0.9.6 due to stat.st_mode type mismatch, so no macOS assets were ever uploaded.

**Execution:**
- Code fix (stat.st_mode as u32) and Cargo version bump were already committed on release/v0.9.7
- Committed contract centralization: canonical gbrain-platform-channel schema in release.yml, install.sh, RELEASE_CHECKLIST.md; macOS CI preflight job; two new test scripts
- Pushed release/v0.9.7 branch and opened PR #83

**Routing memo:** .squad/decisions/inbox/leela-issue79-80-routing.md

**Lesson:** When an installer 404 follows a release, audit the build matrix first — a compile failure upstream is more likely than an asset naming mistake. Professor's D-R79-6 gate (6 criteria) was the correct bar; only gate 6 (real release evidence) remains pending CI.

### 2026-05-01: Dirty tree audit — scribe scripts vs gitignore contract

**What:** Audited 8 untracked files on `main` after compound-term recall PR (#100) was merged.

**Findings:**
- 4 files are dead scribe artifacts (`create_files.py`, `scribe-cleanup.py`, `scribe-commit.bat`, `.squad/git-commit-msg.txt`). The scribe scripts were never executed and are incorrect by design: they attempt to `git add` paths that `.gitignore` explicitly excludes (`.squad/orchestration-log/`, `.squad/log/`, `.squad/decisions/inbox/`). DELETE.
- 4 files are real team knowledge: `.squad/skills/{compound-term-tiered-fts,deterministic-hybrid-proof,search-proof-contracts,search-surface-coverage}/SKILL.md`. These are skill extracts from the compound-term recall work. LAND VIA PR. No OpenSpec required.
- `origin/main` is 1 commit ahead (doc site style #101). Pull before branching.
- `.squad/decisions/inbox/` has 4 committed files that predate the gitignore rule — separate housekeeping PR needed.

**Key pattern:** When scribe scripts appear in the working tree alongside gitignored directories, check `.gitignore` first. Scripts that try to force-add gitignored paths are always wrong.

**Routing memo:** `.squad/decisions/inbox/leela-dirty-tree-audit.md`

---

### Batch 1 scope analysis — vault-sync-engine watcher reliability (2026-04-28)

**Context:** macro88 requested Batch 1 implementation, v0.10.0 release, and ≥90% coverage.

**Key findings:**

- `now.md` gate clause is active: "No next vault-sync slice is active yet; require a fresh scoped gate before implementation resumes." Fry cannot start until Professor signs off on `ReconcileMode::OverflowRecovery`, `WatcherMode` enum, and timer placement.
- Batch 1 = 13 open tasks: 6.7a, 6.8, 6.9, 6.10, 6.11 + tests 17.5w/x/y/z/aa/aaa2/aaa3/aaa4.
- All tests are inline `#[cfg(test)]` modules in the same `.rs` file as the code under test.
- 6.9 → 6.10 → 6.11 is a strict dependency chain via `WatcherMode` + `CollectionWatcherState` struct fields.
- Task 13.6 has a "frozen 13-field" closure note; 6.11 adds three fields. Fry must write a 13.6 addendum before Nibbler review to prevent a reviewer friction block.
- Watcher health fields (`watcher_mode`, etc.) must null-out on Windows in the cross-platform `memory_collections` response — not cfg-excluded.

**Key file paths:**
- `openspec/changes/vault-sync-engine/implementation_plan.md` — Batch 1 spec (lines 36–100+)
- `openspec/changes/vault-sync-engine/tasks.md` — source of truth for task state
- `src/core/vault_sync.rs` — all Batch 1 implementation goes here
- `src/core/reconciler.rs` — `ReconcileMode::OverflowRecovery` variant goes here
- `src/commands/collection.rs` — 6.11 CLI surface (watcher health in `collection info`)
- `.squad/identity/now.md` — active gate clause; read before routing any future batch

**v0.10.0 release gate:** All 13 tasks `[x]`, `cargo test` clean, coverage ≥90%, Nibbler adversarial sign-off on 6.9+6.10, 13.6 addendum written, `Cargo.toml` bumped to 0.10.0, CHANGELOG updated.

**Routing memo:** `.squad/decisions/inbox/leela-batch1-scope.md`

---

### Batch 1 scope repair — Professor rejection enforced (2026-04-28)

**Context:** Professor rejected the `leela-batch1-scope.md` artifact on three grounds. Fry is locked out of the repair. Repaired artifacts and decision record written.

**Three issues repaired:**

1. **6.7a authorization bypass** — The original plan said "add this mode variant to the authorization enum" which would create a new auth bypass. Repaired: `OverflowRecovery` lives in `FullHashReconcileMode` (label only); authorization is `FullHashReconcileAuthorization::ActiveLease { lease_session_id }`. Worker loads `collections.active_lease_session_id`, skips with WARN on null/mismatch. No bypass.

2. **6.11 `memory_collections` widening** — The 13.6 contract is frozen (13 fields, exact-key test). Repaired: 6.11 narrows to `quaid collection info` CLI only. `memory_collections` MCP tool is untouched. MCP widening deferred to a future scoped lane (not Fry under current lockout).

3. **WatcherMode semantics** — `"inactive"` was unreachable given the null rule. Repaired: `WatcherMode` = `Native | Poll | Crashed` only. Non-active collections and Windows → null in CLI output. No `Inactive` variant.

**Key pattern — authorization vs. mode enums (Batch 1 repair):** In `reconciler.rs`, `FullHashReconcileMode` is the operation label (audit trail, test distinguishability). `FullHashReconcileAuthorization` is the proof of who is allowed to run. These are separate. New operation modes are low risk. New authorization variants are high risk and require Professor sign-off. When an implementation plan says "add this variant to the authorization enum," treat it as a red flag requiring explicit review. The safe default is to add the mode to the label enum and reuse an existing authorization token.

**Key pattern — MCP schema freeze (Batch 1 repair):** A "frozen N-field" closure note on an MCP tool means there is an exact-key test asserting that the response object has exactly those N fields. Any addendum that adds fields is a breaking change requiring: (a) update design.md, (b) change the exact-key test, (c) get a fresh reviewer gate. "Documented extension, not a schema violation" is not acceptable framing while the exact-key test asserts otherwise.

**Fry lockout scope:** Fry cannot implement the repaired Batch 1 artifact. Recommended next implementer: Mom.  
**MCP widening (deferred):** When `memory_collections` widening is revisited (Batch 2 or later), it is a NEW scoped slice that must reopen 13.6 explicitly. Not covered by current lockout — any available implementer can be assigned once Professor gates it.

**Routing memo:** `.squad/decisions/inbox/leela-batch1-repair.md`

- **PR #110 guardrails bypass fix — merge_commit_sha is the load-bearing gate (2026-04-28):** `/commits/{sha}/pulls` returns PRs that *contain* the commit in their branch history, not just merged PRs. The old check treated any PR association as proof of PR-path arrival. The correct filter requires four simultaneous conditions: `state=='closed'`, `merged_at` is set, `base.ref=='main'`, AND `merge_commit_sha==sha`. The `merge_commit_sha` check is the bypass-closer: GitHub sets it to the exact commit SHA that lands on main for all three merge strategies (regular, squash, rebase). A directly-pushed commit's SHA cannot match because it is not a merge commit. Vault_sync.rs clippy failures (dead_code on WatcherHandle fields + cmp_owned in test) were outside guardrails scope but were blocking the Check gate on the same PR — fixed with minimal mechanical changes (#[allow(dead_code)] for Drop-held watcher fields; direct string comparison for path equality in test). Both are in the same commit to keep the PR gate-ready in one push. Reviewer lockout enforced: Fry, Mom, Zapp all locked out of this revision cycle.
