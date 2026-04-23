# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- Batch L1 final adversarial review (2026-04-23): **APPROVE** for the narrowed restore-orphan startup lane only. Controlled seams: fresh-heartbeat defer uses the shared 15s gate, ownership truth stays `collection_owners`-scoped after stale-session sweep and own-session claim, startup order is explicit (`registry init -> sweep stale sessions -> register/claim -> RCRT -> supervisor bookkeeping`), and blocked collections stay visibly blocked instead of reporting startup success. Required caveat: this does **not** certify sentinel recovery, generic `needs_full_sync` startup healing, remap attach, or any broader "serve startup heals dirty collections" story.
- Batch L1 pre-implementation gate (2026-04-23): **APPROVE the narrowed restore-startup slice only.** Safe boundary is exactly `17.5ll` + `17.13` + the minimal `11.1` process-global registry work needed for RCRT/supervisor bookkeeping; defer sentinel-directory recovery (`11.4` / `17.12`) and any broader startup dirtiness claims. Adversarial non-negotiables: startup recovery must defer while `pending_command_heartbeat_at` is fresh unless the exact restore originator is calling; ownership truth must stay `collection_owners` scoped to the collection, never ambient/foreign `serve_sessions`; startup order must remain stale-session sweep → own session registration/lease claim → RCRT restore recovery → supervisor spawn; approval proves only restore-orphan recovery (`Deferred`/`OrphanRecovered`/`Finalized` under StartupRecovery) and must not be used to imply generic `needs_full_sync`, sentinel, remap, or wider post-Tx-B startup healing.
- Batch L pre-gate (2026-04-23): **REJECT as proposed.** Startup/orphan restore recovery is not adversarially safe bundled with sentinel startup recovery and optional Windows gating. Safer split: keep the restore-only startup lane together (`17.5ll` + `17.13` + only the minimal `11.1` registry work needed for RCRT/supervisor state) and defer `11.4` sentinel recovery to the separate crash-mid-write proof (`17.12`). Non-negotiables: RCRT must fail closed while the originator heartbeat is fresh, ownership truth must stay `collection_owners` not ambient `serve_sessions`, malformed or unreadable sentinels must leave the dirty signal set, and restore startup recovery must not become a success-shaped excuse for broader `needs_full_sync` / remap / generic post-Tx-B attach claims.
- Batch K2 final adversarial review (2026-04-23): **APPROVE.** Controlled seams: restore originator identity is persisted and only the matching originator bypasses a fresh heartbeat; external CLI finalize fails closed on fresh-heartbeat/deferred, manifest-incomplete, integrity-failed, aborted, and no-pending states; `run_tx_b` remains authoritative for pending-root residue and only reopens through explicit CLI attach completion; manifest retry/tamper now split retryable vs terminal with `restore-reset` blocked unless integrity has actually failed. The `17.11` proof stays honest because it is a real `collection restore` → `collection sync --finalize-pending` CLI handoff through `finalize_pending_restore_via_cli` + `complete_attach`, not a serve/RCRT topology claim. Required caveat: approval is only for the K2 offline-restore integrity slice and its CLI finalize proof; online handshake, serve-startup recovery/orphan finalization (`17.5ll`, `17.10`, `17.13`), and broader destructive restore surfaces remain deferred and must stay described as deferred.
- Batch K1 final review (2026-04-23): **APPROVE.** The adversarial seams named in pre-gate are now acceptably controlled for the narrowed K1 slice: add-time lease ownership + cleanup RAII-backed, probe tempfiles removed on all paths, root/ignore validation before row creation, writable downgrade only on permission-class refusal, shared read-only gate scoped to vault-byte writers. Task ledger explicitly out-of-scope DB-only mutators from K1 claim; no offline restore broadening. Approval covers only narrowed slice (default attach + list/info + vault-byte refusal); does not certify offline restore integrity, RCRT/CLI finalize closure, or broader DB-only blocking. Required caveat attached and must stay explicit for landing documentation. **K1 APPROVED FOR LANDING.**
- Batch K1 final re-gate (2026-04-23): **APPROVE.** Adversarial seams controlled: add-time lease ownership, validation-before-row-creation, probe cleanup, truthful writable=0, vault-byte refusal for gbrain put / brain_put, and restoring-gated slug-bound brain_gap. Blocking seam now controlled; success-shaped leakage prevented; repair narrow; deferral explicit; K1 stays narrowly honest. Caveat: approval covers K1 attach/read-only slice only; does not certify offline restore integrity, RCRT finalize closure, broader DB-only mutator read-only blocking, or K2 destructive-path proof. **K1 APPROVED FOR LANDING.**
- Batch K1 (collection add + shared read-only gate) is adversarially safe only as a narrow attach-surface slice: no offline restore integrity claims, no write-back widening, and success only after root/ignore validation, short-lived lease cleanup, probe cleanup, truthful `writable=0`, and shared mutator refusal proofs all land together. **2026-04-23 Pre-gate:** APPROVE with hard seams: add-time owner lease, probe artifact residue, pre-row root/ignore refusal, writable misclassification, and read-only gate bypasses across CLI/MCP/global write shims.
- Batch J re-gates cleanly once `collection sync --finalize-pending` fails closed for every non-final `FinalizeOutcome`; the slice stays acceptable only if that truth surface remains CLI-only and does not imply deferred MCP or destructive-path proof. **2026-04-23 Final Re-gate:** APPROVE. Previously blocking seam now acceptably controlled: blocked finalize outcomes no longer return exit 0; only `FinalizeOutcome::Finalized` and `FinalizeOutcome::OrphanRecovered` render success. All other finalize outcomes fail closed with `FinalizePendingBlockedError` and explicit wording. No success-shaped behavior leaks; repair narrow; deferral explicit. Required caveat: Approval covers CLI truth seam for Batch J narrowed slice only; does not affirm MCP surfacing, destructive restore/remap paths, or full finalize/integrity matrix as complete. **Batch J APPROVED FOR LANDING.**
- Vault-sync-engine Batch J (2026-04-23): **RECONFIRMED NARROWED SLICE AFTER RESCOPE**. Nibbler's original pre-gate approved narrowed batch only as combined slice with all 18 proof items attached. When Professor proposed rescoping to plain sync + 7-ID closure only, Nibbler reconfirmed: the narrowed split is safe **if** implementation keeps plain sync strictly on active-root reconcile lane and does NOT use it as recovery multiplexer. Current code shape supports it: bare sync still hard-errors, fail-closed gates already exist, destructive paths separate, ownership/lease primitives center on `collection_owners`, restore/remap stays behind restoring + needs_full_sync + RCRT attach. Adversarial non-negotiables reaffirmed: active-root reconcile only, blocked states blocked and truthful, CLI ownership singular, reconcile halts terminal, operator surfaces honest. Fry implementation complete; Scruffy proof lane complete; all decisions merged. Next: implementation gate confirmation before landing.
- Narrowed Batch J is safe to implement next only if bare `collection sync` stays an active-root reconcile entrypoint, never a recovery multiplexer: no implicit finalize, remap, or write-reopen, and no new MCP surface just to report blocked states.
- Once plain sync works, `sync`, `sync --finalize-pending`, `restore-reset`, `reconcile-reset`, and `brain_collections` become one operator trust surface: every command must preserve fail-closed state truth and never imply writes reopened before RCRT or reset preconditions are actually satisfied.
- Batch I re-gates cleanly only when legacy ingest/import honor the same global `state='restoring' OR needs_full_sync=1` interlock, offline restore/remap stop before attach, and the task ledger openly keeps plain sync plus offline CLI end-to-end recovery in deferred territory.
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
- Reconcile apply must distinguish true creates from existing-page updates before raw_import rotation; if an existing page reaches apply with zero total raw_import history, silent bootstrap is identity corruption, not healing.
- Zero-total `raw_imports` is a different seam from zero-active history: the shared rotation helper may still allow first-write bootstrap, so existing-page apply paths need their own pre-mutation guard while truly new pages remain the only narrow row-count-zero bootstrap case.
- Destructive bypass modes are not identity-scoped just because the API carries a string; if code only checks a non-empty `restore_command_id` or lease/session token without comparing it to persisted ownership state, any caller can forge the bypass.
- Batch H re-gates cleanly once restore/remap full-hash authorization compares the caller token to persisted collection owner fields and fails closed on missing or mismatched owners; mode shape plus any non-empty string is no longer enough.
- Batch I is only gateable as one slice if ownership, finalize, reattach, and write-gate land together; splitting them would leave a success-shaped destructive path with no trustworthy owner or reopen barrier.
- `collection_owners` must stay the sole ownership truth; any fallback to `serve_sessions`, `supervisor_handles`, or restore-command residue for live-owner resolution reopens spoofed-release and split-brain restore.
- The `(session_id, reload_generation)` ack is safe only if commands also fail closed on owner change, serve death, stale ack residue, and fresh-serve impersonation; matching one field is not enough.
- `run_tx_b` and RCRT are separate authority boundaries: finalize may happen through the canonical helper, but reattach/open-writes must stay exclusive to the RCRT attach-completion path under single-flight.
- Batch I credibility needs explicit tests for the OR write-gate and RCRT skip-on-halt behavior, even if those tests were not in the initial batch list; otherwise restore/remap can quietly reopen writes or bulldoze integrity blocks.
- Batch I still fails gate if any offline or command path calls `complete_attach` directly; even with `run_tx_b` canonicalized, bypassing RCRT turns `needs_full_sync` into a transient bit instead of the promised reopen barrier.
- Batch K1 is landable only when the task ledger says out loud that `CollectionReadOnlyError` covers vault-byte writers only (`gbrain put` / `brain_put`), while slug-bound `brain_gap` and other DB-only mutators remain on the restoring/needs_full_sync interlock. Add-time safety is acceptable when root and `.gbrainignore` validation happen before row creation, fresh attach runs detached under a short-lived `collection_owners` lease, probe residue is cleaned, and failed attach deletes the just-inserted row instead of leaving a success-shaped stub.
- Batch K2 is safe only as one inseparable offline-restore integrity slice, and only if it includes the live production fixes that make the proofs honest: offline restore must persist and use restore originator identity instead of bypassing straight to `run_tx_b`, `restore-reset` must stop unconditional state erasure, Tx-B residue must stay authoritative, manifest retry/tamper must stay terminally visible, and the end-to-end proof must verify the real CLI→RCRT handoff rather than a fixture shortcut.

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

- Batch J narrowed sync flow still fails gate if any blocked finalize outcome is rendered as CLI success; `--finalize-pending` must not emit exit-0 / `"status":"ok"` for deferred, manifest-incomplete, integrity-failed, or aborted states because that recreates a success-shaped blocked path even while plain no-flag sync stays narrow.
- Batch K is safer split in two: keep restore-integrity proof closure separate from `collection add`/read-only attach surface. The forcing risk is trust-surface coupling: `collection add` introduces a new owner-acquisition path, capability-probe truth, `writable=0` persistence, and future `CollectionReadOnlyError` routing across every mutator; bundling that with offline restore proof can manufacture success-shaped restore confidence while lease residue or read-only misclassification still lets the wrong actor or wrong write path proceed. If `17.11` depends on real add scaffolding, move it with the add slice rather than weakening the restore-proof slice.

## 2026-04-23T09:01:00Z Batch K2 Final Approval

**Verdict:** APPROVE

Adversarial seams reviewed and controlled. Identity theft, reset/finalize dishonesty, Tx-B residue loss, and manifest tamper all acceptably scoped within offline CLI boundary. K2 APPROVED FOR LANDING. Caveat remains: only offline CLI closure approved, not startup/orphan recovery, online handshake, or broader destructive surfaces.
