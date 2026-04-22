# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- Phase 3 review confirmed that raw-data and gap endpoints are only acceptable once payload shape checks, overwrite intent, and transport-size caps are all closed together; one missing seam keeps the whole surface soft.
- Adversarial review begins at the proposal, not only at the code diff.
- This project values hidden-risk discovery and reviewer lockout discipline.
- Local-first systems still need security and misuse thinking.
- Privacy-safe fields are not enough if adjacent free-form fields can still carry the same secret.
- Line-oriented shell protocols need explicit payload caps or raw-data endpoints become an easy memory-pressure path.
- For vault-bound walks, `WalkBuilder` output is only a candidate list; root-bounded `open_root_fd` + `walk_to_parent` + `stat_at_nofollow` must be the only authority for classification if symlink escapes are to stay closed.
- Hash-based rename guards are not safe if they use whole-file byte counts plus a non-empty-body check; conservative pairing needs post-frontmatter body significance, or template notes can inherit the wrong page identity.
- Batch E re-gate closed the hash-rename seam once both sides measured trimmed post-frontmatter body bytes, not whole-file size, and regression coverage pinned both refusal and success boundaries.
- Batch F is gateable when raw-import rotation fails closed on zero-active history inside the same write transaction and delete/quarantine decisions re-query DB-only state at apply time rather than replaying classification snapshots.
- Deferred restore/full-hash and UUID writeback seams are acceptable only when tasks and code comments keep them explicit and error-shaped; success-shaped stubs would make the same slice rejectable.

## 2026-04-15 In Progress

- Conducting final adversarial re-review of Phase 2 graph slice (tasks 1.1–2.5) after Scruffy cycle/self-loop suppression fix (commit `acd03ac`).
- Cross-team status: Professor completed parent-aware tree rendering (commit `44ad720`). Both commits now validated against graph specs. Awaiting Nibbler re-review completion before Phase 2 sign-off.


---

## 2026-04-16: Phase 3 Core Review — Rejection (task 8.2)

**Scope:** brain_gap, brain_gaps, brain_stats, brain_raw, call/pipe failure modes  
**Status:** Completed with REJECTION  

**Blocked artifacts:**
1. `src/mcp/server.rs` — brain_raw contract violation, no size limit, silent overwrites, gap privacy leak
2. `src/commands/pipe.rs` — oversized line handling

**Blocking findings:**
- brain_raw accepts non-object payloads (spec violation)
- No payload size limit (abuse vector)
- Silent replace semantics (data-loss risk)
- brain_gap context unbounded (privacy bypass seam)

**Decision:** nibbler-phase3-core-review.md merged to decisions.md  
**Task 8.2:** Not marked complete; different revision author required (reviewer lockout).

### 2026-04-22 17:02:27 - Vault-Sync Batch E Adversarial Review (Initial Rejection → Approval)

**Initial verdict:** REJECT

**Why initially blocked:**

The conservative hash-rename guard in src/core/reconciler.rs was optimistic for trivial/template notes with large frontmatter and tiny body. A template note with 200+ bytes of frontmatter and a trivially small body (e.g., 'Hi\n') could pass the ≥64-byte size check and be incorrectly paired as a rename, carrying the old page_id onto unrelated content once the apply pipeline lands.

**The exploit:** hash_refusal_reason() checked total file size instead of post-frontmatter body size. Large frontmatter satisfied the byte threshold while the actual human-authored body remained trivial.

**Repair delivered by Leela:**

1. MissingPageIdentity.body_size_bytes = compiled_truth.trim().len() + timeline.trim().len()
2. NewTreeIdentity.body_size_bytes = body.trim().len() (post-frontmatter)
3. hash_refusal_reason() gates on body_size_bytes < 64, not whole-file size
4. Refusal reason strings renamed for clarity

**Re-verdict:** APPROVE

**Why this is sufficient:**

- Note can no longer satisfy 64-byte threshold by stuffing bytes into frontmatter
- Refusal path tested directly at helper boundary
- Classification path tested end-to-end: whole-file-large / body-tiny note → hash_renamed = 0, quarantine
- Surrounding scope remains honest: tasks.md says native pairing is interface-only, apply/hash pipeline deferred

**Key learning for future batches:**

The 64-byte threshold in content-hash identity guards ALWAYS refers to body content after frontmatter delimiter. Whole-file size MUST NOT be used as a proxy. This is consistent with spec language in tasks 5.8a0 and 5.8e.

**Next adversarial focus:**
- Batch F apply pipeline: ensure quarantine semantics are not silently bypassed
- Batch F rename inference: test ambiguous cases stay quarantined (don't flip to false positives)
