# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## 2026-04-25 — Issue #79 installer/release seam early review

**VERDICT: REJECT FOR SHIP until all 6 merge-bar criteria met**

Issue #79 (404 for `gbrain-darwin-x86_64-airgapped`) is not a naming mismatch. The root cause is a failed v0.9.6 release workflow on macOS due to `stat.st_mode` type mismatch in `src/core/fs_safety.rs`. Installer and release naming are already aligned in source. The problem is incomplete shipped releases; a narrow installer fallback would hide the broken release contract.

**Explicit rejection of installer-only 404 patches, manual asset upload, or checklist-only rewording.** The 6-criteria approval bar for v0.9.7:
1. macOS build break fixed + proven on both darwin-x86_64 and darwin-arm64
2. Asset names centralized (mechanically coupled, no drift)
3. Full manifest proof in CI (fail closed unless all 8 binaries + 8 .sha256 present + checksums match)
4. Installer real-release proof (not mocked; exact darwin-x86_64-airgapped seam tested)
5. Review surfaces truthful (RELEASE_CHECKLIST.md and docs use same channel-suffixed names)
6. v0.9.7 tag build completes + GitHub Release has full manifest before announcement

Deliverable: canonicalized release-asset-contract skill at `.squad/skills/release-asset-contract/SKILL.md` for future releases.

## 2026-04-25 — Vault-sync 13.5 read-filter slice

**VERDICT: APPROVE**

All five contract points are closed by the uncommitted diff.`BrainQueryInput`, `BrainSearchInput`, and `BrainListInput` each gain `collection: Option<String>`. All three MCP handlers call a single thin `resolve_read_collection_filter_for_mcp` → `collections::resolve_read_collection_filter`, which correctly implements the default rule: explicit name → `NotFound` error if absent; exactly one `state = 'active'` row (fetched with `LIMIT 2` to short-circuit correctly) → use it; otherwise → write-target via `is_write_target = 1`. Unknown-collection error is code `-32001` with message `"collection not found: X"`, tested for all three tools in one combined test. Explicit-filter isolation is tested independently per tool. Default-rule behavior is proven by two direct-evidence tests (`brain_search_defaults_to_single_active_collection` for the sole-active branch; `brain_query_defaults_to_write_target...` + `brain_list_defaults_to_write_target...` for the write-target fallback). The `collection_filter: Option<i64>` parameter is threaded all the way through `fts.rs`, `inference.rs`, and `search.rs` without introducing a new CLI surface: both `commands/query.rs` and `commands/search.rs` pass hard-coded `None`. No write-path changes, no 17.5aa5 widening, no slug-resolution cluster claims.

## 2026-04-25 — Vault-sync 13.6 + 17.5ddd re-review (spec-boundary correction)

**VERDICT: APPROVE**

The spec boundary correction is sufficient. `parse_ignore_parse_errors` (vault_sync.rs:694) filters the stored `ignore_parse_errors` JSON to `code == "parse_error"` only and collapses an empty result to `None`/null. A DB row holding `file_stably_absent_but_clear_not_confirmed` therefore returns `null` at the MCP layer — exactly what design.md §505 now specifies for task 13.6. The behaviour is not merely asserted in design text: the test at server.rs:3379/3450 plants the stable-absence entry in the DB and directly asserts `ignore_parse_errors` is null on the response. 17.5ddd's frozen-schema test enumerates all 13 required response fields by exact key equality, matching the design.md schema verbatim. The deferred arm (17.5aa5) is explicitly named in both tasks.md and design.md and is not smuggled or overclaimed here. Seam is coherent; slice approved for landing.

## Learnings

- PR #104 search-skill preservation review (2026-04-27): **APPROVE FOR LANDING** when a metadata-only PR adds reusable squad skills as standalone artifacts and keeps any agent-history note as truthful provenance rather than smuggling behavioral claims. Pending CI is not a blocker by itself when the diff does not touch product code or public contracts.
- Issue #79 installer/release seam (2026-04-25): **REJECT any installer-only 404 patch.** When `install.sh` and `release.yml` already agree on channel-suffixed names, a missing asset is usually a failed release-manifest closure, not a naming typo. The approval bar is one canonical asset manifest, exact release-manifest verification, and a real tagged release proving every promised platform/channel asset exists before calling the version shippable.
- PR #77 feedback collapse (2026-04-25): repeated review noise around one branch can hide the real merge bar. Separate comments into (1) doc-truth drift that is satisfiable in prose, (2) genuine code defects like active/read-surface leaks, and (3) policy disputes already closed by prior gate decisions such as the intentional Unix gate on `gbrain serve`.
- Vault-sync quarantine-restore pre-gate (2026-04-25): re-enabling a deferred destructive surface is landable only when the reopened contract names both stable end states and proves the hard parts directly — install-time no-replace semantics plus parent-fsynced post-unlink cleanup. Converting deferred-surface tests into precise happy/failure contract proofs is mandatory; retaining only broad "restore failed" coverage is not enough.
- Vault-sync Batch 13.6 / 17.5ddd review (2026-04-24): **REJECT for landing** when `brain_collections` computes terminal `integrity_blocked` from `reconcile_halt_reason` alone instead of the frozen `reconcile_halted_at IS NOT NULL AND reason` predicate. Read-only shape work is not enough if the presentation can overstate a terminal halt from stale metadata; schema-fidelity reviews must verify the exact truth predicate, not just field names.
- Vault-sync Batch 13.3 review (2026-04-24): **REJECT for landing** when CLI slug-parity canonicalizes only success paths and leaves a resolved failure path speaking raw inputs. If a command has already resolved `from`/`to` to collection-aware identities (for example `gbrain unlink`), even "no matching link found" must emit canonical `<collection>::<slug>` addresses or the CLI parity claim is still incomplete.
- Vault-sync Batch N1 review (2026-04-24): **REJECT for landing despite correct MCP truth surface** when a supposedly MCP-only slug-routing slice silently widens shared CLI behavior. Shared-helper reuse is not a scope exemption: if `src/commands/check.rs` changes single-page `gbrain check` resolution/filtering semantics to support `brain_check`, the batch must either narrow the implementation back to MCP-only or state the CLI widening explicitly as its own reviewed surface.
- Vault-sync Batch L1 final review (2026-04-23): **APPROVE FOR LANDING**. The slice stays inside the approved boundary: registry-startup scaffolding plus restore-orphan startup recovery only. `src/core/vault_sync.rs` keeps one shared 15s stale threshold (`SESSION_LIVENESS_SECS`) across stale-session sweep, owner-liveness checks, and fresh-heartbeat defer; startup order is real (`sweep_stale_sessions -> claim_owned_collections -> run_rcrt_pass -> sync_supervisor_handles`, then runtime thread/spawn bookkeeping); and tests prove fresh-heartbeat defer, stale-owner takeover over foreign residue, exact-once orphan finalize, and no leftover supervisor-ack residue. Required caveat remains explicit: `11.1b`, `11.4`, `17.12`, and any IPC/online-handshake widening are still deferred and must not be implied by this approval.
- Vault-sync Batch L1 pre-gate (2026-04-23): **APPROVE the rescoped L1 boundary** only as restore-orphan startup recovery: `11.1` split to registry-only (`11.1a`) plus `17.5ll` and `17.13`. Non-negotiables: explicit startup order `registry init -> RCRT -> supervisor spawn`, fatal registry-init failure, one shared 15s stale-heartbeat threshold, canonical `finalize_pending_restore(..., FinalizeCaller::StartupRecovery { .. })` + attach seam only, fresh-heartbeat defer behavior, and no success-shaped outcome unless startup finalize+attach actually completes. Sentinel-directory init, `11.4`, `17.12`, Windows gating, and any handshake/IPC widening stay in L2.
- Vault-sync Batch L pre-gate (2026-04-23): **APPROVE only as the startup-recovery closure slice** — process-global registry init + startup sentinel sweep + startup-owned RCRT orphan finalize/attach. Keep the batch narrowly about "serve starts after the originator died" and require one explicit startup order (`registry init -> sentinel sweep/dirty flag -> RCRT finalize/attach -> supervisor spawn`) plus one explicit stale-command threshold (15s = three missed 5s heartbeats). Defer Windows platform gating and any expansion of live online handshake/IPC semantics unless they can be proved without widening the restore state machine.
- Vault-sync Batch K2 pre-gate (2026-04-23): **APPROVE K2 as the next boundary only if it stays an offline restore-integrity closure batch.** K1 removed the attach/read-only scaffolding blocker; what remains is one coherent state-machine slice: persist offline `restore_command_id`, compare exact originator identity in `finalize_pending_restore()`, keep Tx-B residue durable, make manifest incomplete vs tamper terminal states operator-truthful, and prove one real CLI end-to-end offline restore completion path. If implementation drifts into online handshake work, broader read-only widening, or invents a second completion topology beyond the explicit offline path, split again rather than hiding that expansion inside `17.11`.
- Vault-sync Batch K1 final review (2026-04-23): **APPROVE the narrowed K1 boundary** as fresh-attach + read-only scaffolding. All adversarial seams now acceptably controlled: add-time lease ownership + cleanup, probe artifact refusal, root/ignore validation before row creation, writable misclassification, and read-only gate scope. K1 stays narrowly honest — `collection add/list` plus vault-byte refusal (`gbrain put` / `brain_put`) only; DB-only mutators (`brain_gap`, `brain_link`, `brain_check`, etc.) remain on restoring interlock. No K1 item needs move to K2, but task ledger must explicitly exclude `9.2a` write-back behavior and defer offline restore integrity to K2. Required caveat: K1 is default attach + list/info truth + vault-byte refusal only; `--write-gbrain-id`, broader root-byte mutators, and restore-integrity closure deferred to later batches. Final approval issued; boundary preserved; caveats attached. **K1 APPROVED FOR LANDING.**
- Vault-sync Batch K1 pre-gate (2026-04-23): **APPROVE the narrowed K1 boundary** only as fresh-attach + read-only-gate scaffolding. Non-negotiables: `collection add` must validate root/name/ignore state before row creation, run fresh attach through `FullHashReconcileMode::FreshAttach` with `AttachCommand` authorization (not an active-lease shortcut), keep default attach read-only, and implement `CollectionReadOnlyError` as a shared helper for root-byte mutators only — not DB-only tools like `brain_gap`, links, assertions, or raw-data writes. No K1 item needs to move back to K2, but `9.2` must be read as default attach only; do not quietly smuggle `9.2a` write-back behavior into this slice without its own honest proof.
- Vault-sync Batch K pre-gate (2026-04-23): **REJECT proposed combined boundary.** `collection add` is still a brand-new operator surface (`src/commands/collection.rs` has no add/list path yet), while the "offline restore integrity matrix" still hides real production gaps rather than pure proofs: offline `begin_restore()` does not persist `restore_command_id`, `restore_reset()` is still unconditional, and `writable=0` / `CollectionReadOnlyError` are not enforced. Safer split: land `1.1b`, `1.1c`, `9.2`, `9.2b`, `9.3`, `17.5qq10`, `17.5qq11` first as the honest scaffolding slice; then take `17.5kk3`, `17.5ll3`, `17.5ll4`, `17.5ll5`, `17.5mm`, `17.11` as a separate destructive-path closure batch with mandatory Nibbler focus on restore identity persistence, manifest verification, and Tx-B residue.
- Vault-sync-engine Batch J (2026-04-23): **FINAL RE-GATE APPROVED FOR LANDING**. Failed original Batch J (too large, proof-only misclaim), proposed narrower boundary: plain sync + reconcile-halt safety only. Reconfirmed narrowed batch is safe: hard-error sync still extant, fail-closed gates preserved, destructive paths separate, all non-negotiables affirmed. Fry implementation complete; Scruffy proof lane complete; all 6 decisions merged. **2026-04-23 Final Re-gate:** APPROVE. Blocked finalize outcomes now fail closed with explicit wording and non-zero exit. Only `FinalizeOutcome::Finalized` and `FinalizeOutcome::OrphanRecovered` render success. CLI truth sufficient for narrow repair. Tasks.md honest (active-root only, broader finalize/remap/MCP deferred). Caveat: Batch J remains CLI-only proof point; broader finalize/integrity matrix deferred. **Batch J APPROVED FOR LANDING.**
- This team expects explicit reviewer gating, not silent approval.
- Maintainability and architectural coherence are key review criteria.
- For CLI review, validate behavior from more than one working directory; path-dependent “embedded” resources can look correct at repo root while failing the shipped-binary contract.
- A schema-foundation slice is not landable if it bumps required page fields or uniqueness rules without updating downstream insert/query callsites and quarantine filters; `cargo check` can stay green while runtime tests collapse.
- A repair pass is still rejectable when `tasks.md` is updated but proposal/design artifacts continue to describe a different contract; reviewer truthfulness is proposal-first, not task-note-first.
- Full green tests do not clear a schema slice if legacy-open paths still mutate an old database before refusing it; preflight safety must happen before any v5 DDL side effects.
- When a foundation slice keeps a temporary compatibility shim, proposal/design text must say so explicitly; a repair note in tasks.md is not enough to clear truthfulness review.
- A batch can be truthful about partial implementation and still be rejectable if a public scaffold on a safety-critical path returns benign success values instead of making deferral explicit.
- For vault-sync work, stubbed reconciler entry points should fail loudly or stay clearly unwired; returning empty stats or `false` for DB-only-state checks is too easy to mistake for real behavior.
- A narrow repair is sufficient to clear a blocker when the scaffold remains explicitly unwired, the task text no longer overclaims replacement, and any safety-critical stub now errors loudly instead of returning benign success.
- Vault-sync foundation stubs on authoritative recovery paths (`full_hash_reconcile`, main `reconcile`) are still rejectable when they return zeroed success stats; unwired safety paths must error loudly, and checked tasks must not claim `WalkBuilder`/rehash delivery before code exists.
- A re-gate can approve an unfinished foundation scaffold when every safety-critical entry point now fails explicitly, the task ledger names deferred walk/hash work honestly, and Unix-only imports are wired so the code is structurally ready for real Unix builds.
- A technically solid vault-sync slice is still rejectable when `tasks.md` mixes stale repair notes with current-state claims; the ledger must describe today's behavior, not preserve contradicted historical status inline.
- A narrow re-gate can clear a prior task-ledger blocker when the current-state note explicitly supersedes the historical repair note and each repaired claim is directly traceable to real code paths and tests.
- A UUID-identity slice is landable when `Page.uuid` becomes mandatory in typed read paths, UUID generation/adoption stays explicit and read-only by default, and task notes clearly defer frontmatter write-back and watcher-native production work.
- A reconciliation apply slice is landable when raw-import rotation is shared across every in-scope writer, DB-only-state is re-checked inside the apply transaction, and later restore/full-hash seams stay explicitly deferred instead of being papered over.
- A full-hash recovery slice is landable only when its API makes authorization explicit, separates stat self-heal from content-changing reingest, and treats zero-active `raw_imports` as a typed invariant failure rather than a recoverable convenience case.
- A follow-on full-hash gate is landable when the unchanged-hash path is provably metadata-only, the changed/new path reuses the existing atomic apply seam, and the task ledger names `brain_put` UUID preservation as a render seam rather than pretending passive reconcile now writes files.
- Vault-sync Batch K1 review (2026-04-23): approve when the slice stays narrowly honest — `collection add/list` plus fresh-attach/read-only scaffolding, with `CollectionReadOnlyError` enforced only on vault-byte writers (`gbrain put` / `brain_put`) while DB-only mutators remain on the restoring interlock. K1 must keep the caveat explicit that `--write-gbrain-id`, broader root-byte mutators, and restore-integrity closure are still deferred, even if the default test lane is green.
- A restore/remap pre-destruction slice is landable only when the drift-capture bypass is authorized by explicit caller identity (lease or restore-command token), the trivial-content predicate is shared with rename resolution, and writes stay blocked until attach completion clears `needs_full_sync`.
- A closed authorization enum is still too weak for destructive safety bypasses if the supplied token is only checked for presence; the code must compare it to persisted owner identity, not just caller class.
- A narrow repair clears a destructive-bypass blocker once authorization compares caller identity to persisted collection ownership before any root walk, keeps fresh-attach on its own seam, and adds direct match/mismatch tests in every supported validation lane.
- The next restore/remap batch is coherent only if it lands the lease/handshake, canonical finalize path, RCRT attach handoff, and write-gate together; splitting those seams leaves destructive recovery either unreviewable or falsely "done."
- A restore/remap batch is still rejectable if any compatibility writer bypasses the new OR write-gate or if a checked task claims a CLI recovery/sync path that still hard-errors; destructive-path review needs both contract closure and truthful operator surface claims.
- A restore/remap repair is landable once legacy compatibility writers share the same OR write-gate, offline restore/remap stop at Tx-B plus `needs_full_sync`, and the task ledger keeps CLI→RCRT attach proof explicitly deferred.
- When a proposed vault-sync batch mixes a new ordinary operator path with destructive-path proof closure, split it unless every listed "proof" item is already implemented; missing error surfaces and operator diagnostics mean the batch is still changing behavior, not just proving it.
- The plain-sync follow-up is coherent once it is confined to active-root reconcile plus lease/terminal-halt honesty; keep handshake/reopen/finalize identity closure in the later destructive-path batch and do not smuggle new MCP surfaces into the narrower slice.
- A narrowed plain-sync batch is still rejectable if adjacent `collection sync` recovery modes return exit-0 / `"status":"ok"` for deferred or integrity-blocked outcomes; blocked finalize paths must stay non-success-shaped even when the no-flag reconcile entrypoint is clean.
- A narrow re-gate can clear that plain-sync blocker once `sync --finalize-pending` treats every non-final `FinalizeOutcome` as a hard error, only success-shapes `Finalized`/`OrphanRecovered`, and the task ledger keeps the CLI-only boundary explicit.


## Core Context

**Historical Summary (2026-04-13 to 2026-04-21):**
- Phase 1 review leadership: Set review bar for truthfulness, maintainability, coherence
- T14–T19 rejection: Exposed inference shim semantic contract drift, embed CLI mixed-mode, build breakage
- Phase 2 graph re-review: Approved after directionality/output/coverage blockers cleared
- Vault-sync foundation third-pass: Identified legacy-open safety gap (v5 DDL before version check), schema compatibility shims, quarantine filtering consistency

**Review Standards Established:**
- Artifact truthfulness: Proposal/design must match implementation exactly
- Safety-first architecture: Preflight checks before mutations; explicit error semantics for safety-critical paths
- Coverage depth: 90%+ required; foundation slices need direct unit tests, not just e2e validation
- Multi-dimensional review: Truthfulness + performance + coverage; don't conflate test-pass with gate-pass
## 2026-04-14 Update

- Fry completed rust-best-practices skill adoption recommendation. Skill recommended for all Rust implementation and review work. Key alignment: error handling split matches our practice, CLI discipline aligns with CI gates, performance constraints match single-binary target.
- MCP evaluation still pending. Coordinator has flagged GitHub MCP as the only currently useful integration for this repo in interim.
- Team memory synchronized: decisions inbox merged into canonical ledger, orchestration logs written, team coordination complete.
- The Rust handbook at `.agents/skills/rust-best-practices/` is adoptable as standing guidance only if rules are classified into defaults vs optional techniques.
- For GigaBrain, strong Rust defaults are borrow-over-clone, `Result` over panic, measured performance work, and justified `#[expect(clippy::...)]` instead of blanket lint suppression.
- GigaBrain is currently a binary crate with internal modules (`src/main.rs`, `src/commands/`, `src/core/`), so library-only rules like blanket `#![deny(missing_docs)]` should stay conditional rather than repo-wide policy.

## 2026-04-14T03:59:44Z Scribe Merge (T03 completion)

- Reviewed T03 markdown slice completion and Scruffy's test strategy.
- T03 decisions merged into canonical `decisions.md`: frontmatter canonical order (alphabetical sort for deterministic render), timeline sep omit-when-empty, YAML parse graceful degradation, non-scalar YAML skip.
- Rust skill adoption finalized in team memory with caveats section (MSRV ≥1.81 for `#[expect]`, nightly-only for rustfmt import grouping, snapshot testing deferred to Phase 1).
- Cross-agent history updated. Orchestration and session logs written. Inbox cleared.

## 2026-04-14 Search/Embed Review

- Phase 1 search review outcome: reject for landing until semantic search stops pretending the SHA-based embedding shim is the specified Candle/BGE implementation.
- Embed CLI review rule clarified: `gbrain embed [SLUG | --all | --stale]` must behave as an explicit mode surface, not a permissive mixture that silently ignores flags.
- Review bar reaffirmed: a slice under active signature churn is not review-complete if targeted `cargo test` compilation no longer passes.

## 2026-04-14T04:56:03Z Phase 1 T14–T19 Code Review (REJECTION)

- Reviewed Fry's T14–T19 submission (commit 2d5f710) for Phase 1 search/embed/query surface.
- **Verdict: REJECTION FOR LANDING** on three blocking grounds:
  1. **Inference shim semantic contract drift:** `inference.rs` public API promises BGE-small-en-v1.5 embeddings but delivers SHA-256 hash shim. No candle-* wiring, no embedded weights, no feature gates. Vector similarity scores are hash-proximity. Semantic guarantees misleading.
  2. **Embed CLI mixed-mode allowed:** Contract says `gbrain embed [SLUG | --all | --stale]` (mutually exclusive). Implementation allows mixed modes (`SLUG + --all`), silently privileges slug path instead of rejecting. Also, `--all` re-embeds instead of skipping unchanged content per spec.
  3. **Tests fail compilation:** `embed::run` signature changed to 4 args. Old test callsites still call 3-arg form. `cargo test` fails before review can proceed.
- **Non-blocking note:** `--depth` flag exposed but ignored; help text should clarify deferral or remove from Phase 1 surface.
- FTS implementation (T13) itself is acceptable. Rejection is on semantic-search truthfulness, embed CLI contract integrity, and build breakage.
- Recommended: Fry address blockers and resubmit, or defer semantic search blocker to Phase 2 if time-bound.
- Orchestration log written: `2026-04-14T04-56-03Z-professor-rejection-findings.md`.
- Leela revision cycle assigned (independent of Fry). Outcome: APPROVED with explicit placeholder caveats + stderr warnings + honest status notes (5 decisions). All 115 tests pass unchanged. Approved for Phase 1 ship gate.

## 2026-04-15 SG-3 / SG-4 / SG-5 Sign-off

- Reviewed the Phase 1 proposal/design before execution, then re-verified gates directly against the tree and binaries.
- SG-3 APPROVED: import/export/re-import roundtrip over `tests/fixtures/` preserved page count and slug set exactly (5 pages, zero semantic diff by gate definition).
- SG-4 APPROVED: `src/mcp/server.rs` exposes exactly the 5 Phase 1 tools; `cargo test mcp` passed; live stdio session completed `initialize`, `tools/list`, and `tools/call` for all 5 tools.
- SG-5 APPROVED: `target/x86_64-unknown-linux-musl/release/gbrain` exists and is genuinely static (`file`: `static-pie linked`; `ldd`: `statically linked`).

## 2026-04-15 Phase 2 Graph Slice Re-review (Final)

- **Status:** RE-REVIEW COMPLETE — APPROVED FOR LANDING
- **Scope:** OpenSpec `p2-intelligence-layer` graph slice only (tasks 1.1–2.5; `src/core/graph.rs`, `src/commands/graph.rs`, `tests/graph.rs`)
- **Timeline:** Initial rejection (prior to phase2 kickoff); Leela revision completed; re-review 2026-04-15
- **Verdict:** All three blockers from the prior rejection are now resolved:
  1. **Directionality contract:** `neighborhood_graph` confirmed outbound-only per spec. `gbrain backlinks` remains inbound surface. Command/API surfaces are now orthogonal.
  2. **Human-readable output:** CLI prints `→ <edge.to> (<relationship>)` over outbound-only result set only. Root no longer appears as self-neighbour.
  3. **CLI test coverage:** `run_to<W: Write>` refactor makes output injectable. Integration tests now capture actual text and `--json` output shape.
- **Temporal gate update:** D2 from Leela revision confirmed in contract — active filter now gates both `valid_from <= date('now')` and `valid_until >= date('now')`. Mom's future-dated link edge case is now covered.
- **Scope caveat:** This approval is for graph slice tasks 1.1–2.5 only. Issue #28 progressive-retrieval budget/OCC review lane remains separate and not re-opened.
- **Validation:** `cargo test graph --quiet` ✅, `cargo test --quiet` ✅, `cargo clippy --quiet -- -D warnings` ✅, `cargo fmt --check` ✅
- **Decision:** APPROVE FOR LANDING to `phase2/p2-intelligence-layer`.

## 2026-04-15 Cross-team Update

- **Scruffy completed graph cycle/self-loop render suppression** (commit `acd03ac`). Self-edges and cycles no longer print root back into human output. Traversal termination unchanged; human-facing contract now matches spec. All validation gates pass.
- **Fry advancing slices:** Progressive retrieval (tasks 5.1–5.6) and assertions/check (tasks 3.1–4.5) both implemented. All 193 tests pass (up from 185). Token-budget logic and contradiction dedup verified. Awaiting Nibbler's final graph re-review before Phase 2 sign-off.

---

## 2026-04-22 Vault-Sync Foundation Third-Pass Review

**Status:** REPAIR DECISION ISSUED

**Scope:** Vault-sync-engine foundation slice (tasks 1.1–2.6) repaired by Leela for schema coherence, legacy compatibility, and test failures.

**Work Performed:**

1. **Artifact Cross-Validation:** Re-read proposal/design against implementation to detect overstated removals
   - Proposal still describes `gbrain import` and `ingest_log` as removed
   - Implementation retains both as temporary compatibility shims
   - This is a valid technical choice, but artifacts must be explicit

2. **Legacy-Open Safety Audit:** Traced schema version checking in `db.rs`
   - `open_with_model()` calls `open_connection()` which executes v5 DDL BEFORE version check
   - Pre-v5 databases can be partially mutated before re-init refusal
   - Preflight safety must happen before ANY v5 execution

3. **Coverage Depth Assessment:** Identified three new branchy seams without direct tests
   - Collection routing matrix (`parse_slug()` with 6 operation types)
   - Quarantine filtering (quarantined pages excluded from vector search)
   - Schema refusal branch (pre-v5 brains rejected before mutations)

**Findings:**

- ✅ Repairs resolve 181 prior test failures (cargo test now passes)
- ✅ Legacy write paths work with new schema
- ✅ Quarantine filtering wired through vector search
- ⚠️ Proposal/design truthfulness — GATE 1
- ⚠️ Legacy-open safety reordering — GATE 2
- ⚠️ Coverage depth for new seams — GATE 3

**Required Before Landing:**

1. **Gate 1:** Align proposal/design with actual transitional contract (keep shims OR remove now)
2. **Gate 2:** Reorder schema gating: version check before ANY v5 DDL
3. **Gate 3:** Add three focused unit-test groups for new seams

**Key Learning:** Schema foundation slices cannot land with green tests alone. Truthfulness (proposal vs implementation), safety (preflight gating), and coverage depth (new seams directly tested) are co-equal gates.

**Verdict:** REPAIR DECISION ISSUED. Three gates remain before landing.

**Decision Artifacts Merged:**
1. Vault-Sync Foundation Review Gating — three-gate policy for future review passes
2. Coverage Depth Review — Scruffy assessment of new branchy seams
3. Professor re-review — artifact truthfulness + safety + coverage findings

**Next Steps:** Leela addresses gates 1–3; resubmits; Professor conducts final review.



## 2026-04-16: Phase 3 Core Review — Rejection (tasks 8.1)

**Scope:** validate.rs, skills.rs, call.rs, pipe.rs, Phase 3 MCP handlers  
**Status:** Completed with REJECTION  

**Blocked artifacts:**
1. `src/commands/validate.rs` — missing stale-vector resolution check
2. `src/commands/skills.rs` — incorrect embedded-vs-local skill resolution

**Acceptable:** call.rs dispatch, pipe.rs continuation, Phase 3 MCP tools  

**Decision:** professor-phase3-core-review.md merged to decisions.md  
**Task 8.1:** Not marked complete; revision author must address blockers and resubmit.

---

## 2026-04-22: Vault-Sync Foundation Rejection → Repair Cycle

**Session:** Professor review → Leela repair of vault-sync-engine foundation slice.

**What happened:**
- Reviewed Leela's vault-sync foundation slice for schema v5 + collections module coherence.
- **Verdict: REJECTION FOR NEXT BATCH** on four blocking grounds:
  1. Task completion overstated in tasks.md vs actual schema
  2. Legacy schema version handling unsafe (executes v5 DDL before legacy check)
  3. Schema changes not integrated with existing write paths (181 test failures in full `cargo test`)
  4. Foundations not yet maintainable (missing quarantine filtering in search_vec, incomplete validator coverage)
- Recommended Leela take ownership of integration-focused repair pass rather than Fry rewriting.

**Repair outcome (Leela completed):**
- All four blockers resolved in coordinated repair:
  1. tasks.md updated to reflect actual schema state (1.1, 1.6, 2.6 marked pending)
  2. Schema version gating fixed: legacy check now runs BEFORE v5 DDL execution
  3. All 20+ legacy write sites now work with NEW unique constraint via `DEFAULT 1` + `ensure_default_collection()`
  4. Quarantine filtering wired through `search_vec` (FTS5 already had it)
- Result: 181 test failures → **0 failures**, foundation ready for follow-on implementation.

**Review lesson:** Rejection + repair cycle is faster than rewrite when the core issue is integration (wiring paths, not design). Gave Leela clear blocker list → she fixed atomically → no rework needed. Schema v5 foundation now coherent and test-clean.

## 2026-04-22 Batch B Narrow Repair Gate Clear

**Session:** Scribe decision merge + Leela narrow repair completion logging

**Review outcome:**
- Professor's gating feedback on Batch B (safety-critical reconciler semantics + documentation accuracy) was resolved via focused repair pass by Leela.
- Repair scope: strict reconciler scaffold surface (reconciler.rs, tasks.md). No Batch C logic, no expand of approved groups.
- Safety semantics fix: has_db_only_state() now returns explicit Err instead of Ok(false), forcing caller error handling.
- Documentation fix: module header now accurately describes "will replace" (future) vs. "replaces" (completed).

**Decision ledger:**
- Leela's three repair decisions merged to canonical decisions.md (gate decision, repair decision, original review decision now in record)
- Decisions inbox cleared; Scribe orchestration/session logs written

**Batch B status:** ✅ Gate clean, ready for Batch C implementation planning. Professor can now sign off on Group 3 (ignore_patterns), Group 4 (file_state), and Group 5.1 scaffold landing.


## 2026-04-22 Vault Sync Batch C — Final Re-gate (Approved)

**Session:** Professor final gate authority after Leela repair and Scruffy coverage validation.

**Progression:**
1. **Initial REJECT:** Missing Unix imports + overclaimed tasks (2.4c, 4.4, 5.2 marked complete when only scaffolding existed).
2. **Leela repair:** Added conditional imports, demoted tasks, fixed docs. Focused, conservative fix.
3. **Scruffy validation:** Direct test coverage on seams; explicit error contracts on safety-critical stubs.
4. **Final re-gate:** APPROVE.

**Why it clears:**
1. **Prior safety blocker resolved:** Safety-critical scaffold no longer returns benign success values. econcile(), ull_hash_reconcile(), and has_db_only_state() all fail loudly instead of silently.
2. **Task truthfulness restored:** Deferred walk/hash/apply behavior no longer claimed complete. Checked items are foundation-only; unchecked items remain pending.
3. **Unix-compile honesty repaired:** Conditional imports in place. ustix wired under cfg(unix) in Cargo.toml. Code structurally ready for Unix builds (local validation has no Linux target available; cross-compilation check skipped but import fixes are correct).
4. **Validation green:** cargo test --quiet ✅; cargo clippy --quiet -- -D warnings ✅

**Verdict:** Ready to land as explicitly unwired foundation. Honest about deferral. Loud on safety-critical paths. Maintainable for next batch.

**Next:** Batch D (full reconciler walk) has clear handoff. Fd-relative primitives in place, stat helpers functional, platform gates protect invariants. Walk plumbing, rename resolution, delete-vs-quarantine classifier ready to wire.


### 2026-04-22 17:02:27 - Vault-Sync Batch E Gate Review

**Gate verdict:** APPROVE

**Why it clears:**

1. **UUID / gbrain_id wiring is truthful for this slice:**
   - parse_frontmatter() preserves gbrain_id
   - render_page() re-emits it when present
   - ingest/import adopt frontmatter UUIDs or generate UUIDv7 server-side
   - put / MCP write paths resolve persisted identity explicitly (no placeholders)

2. **Page.uuid is non-optional at the type seam:**
   - Page struct requires uuid: String
   - Typed read paths fail loudly on NULL rows (no fabricated defaults)
   - All 15+ Page construction sites audited and updated

3. **Default ingest remains read-only on source bytes:**
   - Compatibility ingest/import path stores generated UUIDs only in pages.uuid
   - Tests prove source markdown unchanged
   - Git worktree stays clean

4. **Rename classification is conservative and correctly staged for Batch E:**
   - Native rename pairs apply through explicit interface seam only
   - UUID matching works correctly
   - Guarded hash matching includes INFO refusal logging on ambiguous/trivial cases

5. **tasks.md is honest about the boundary:**
   - Checked items describe implemented classification/identity slice
   - Watcher-produced native events explicitly deferred
   - Apply-time quarantine/create mutations explicitly deferred
   - brain_put/admin write-back explicitly deferred

6. **Coverage is sufficient to merge this slice:**
   - Direct tests on gbrain_id parse/render/import round-trips
   - Read-only ingest behavior proven
   - Non-optional Page.uuid seam covered
   - Native/UUID/hash rename boundaries tested
   - cargo test --quiet: 439 tests pass
   - cargo clippy --quiet -- -D warnings: clean

**Landing note:** This is a narrow Batch E identity/reconciliation slice, not full write-back or watcher-native completion. Remaining work is clearly isolated in later tasks rather than hidden behind permissive defaults.

**Next review focus:**
- Batch F apply pipeline must preserve quarantine classifications
- Batch F full_hash_reconcile must use identity from Batch E
- Later: Batch F raw_imports rotation and GC

## 2026-04-23 Vault-Sync Batch F Gate Review

**Gate verdict:** APPROVE

**Why it clears:**

1. **Atomic raw-import rotation is real on the in-scope paths.**
   - `core::raw_imports::rotate_active_raw_import()` is now the shared rotation seam.
   - `commands::ingest`, `core::migrate::import_dir`, and reconciler apply-time reingest all invoke it inside the same SQLite transaction as their page/file-state writes.
   - The reconciler also enqueues `embedding_jobs` in that same transaction, matching the Batch F contract.

2. **Active-row invariants now fail loudly where Batch F actually writes.**
   - Rotation refuses any page that already has raw-import history but zero active rows, surfacing `InvariantViolationError` instead of silently healing corruption.
   - Post-rotation assertions keep every exercised write path at exactly one active row.
   - The remaining restore / `full_hash_reconcile` caller hookup is still explicitly deferred rather than misrepresented as done.

3. **Delete vs quarantine is re-evaluated at apply time.**
   - `apply_delete_or_quarantine()` re-checks all five DB-only-state branches inside the transaction that mutates the page/file_state rows.
   - Tests cover both the stale-classification seam and each preservation branch, so the reconciler no longer trusts an earlier snapshot.

4. **Batching and task truthfulness are acceptable for landing.**
   - Apply work is staged into explicit 500-action transactions with a regression test proving the first chunk commits even if a later chunk fails.
   - `tasks.md` clearly marks Batch F complete items versus deferred restore/full-hash/write-through work, which keeps the review boundary honest.
- Vault-sync Batch K2 final review (2026-04-23): **APPROVE FOR LANDING**. The K2 slice stays inside the approved offline restore-integrity closure: offline `begin_restore()` persists `restore_command_id`, `finalize_pending_restore()` only bypasses the fresh-heartbeat gate for the matching originator token, `run_tx_b()` leaves durable pending residue on failure, manifest-missing retries escalate to `integrity_failed_at`, tamper stays terminal until `restore-reset`, and `sync --finalize-pending` now drives the real CLI attach path (`finalize_pending_restore_via_cli` → `complete_attach`) proven by the end-to-end truth test. Required caveat remains explicit: this approval covers the offline CLI closure only; startup/orphan recovery, online handshake, and broader post-Tx-B topology are still deferred and must not be implied by `17.11`.

## 2026-04-23T09:00:00Z Batch K2 Final Approval

**Verdict:** APPROVE

Offline CLI closure meets all gating criteria. Tx-B residue, originator identity, reset/finalize surfaces all truthfully proven. Startup/orphan recovery and online handshake deferred to K3+. K2 APPROVED FOR LANDING.

## 2026-04-24 M1b-i/M1b-ii Session — Final Review in Progress

- **M1b-i proof lane COMPLETE (Bender):** Write-gate restoring-state proof closure (tests-only). No production code changes. Found no missing behavior — all mutators already call `ensure_collection_write_allowed` before mutation. 11 write-gate assertions (6 new + 5 pre-existing), all passing.
- **M1b-ii implementation lane COMPLETE (Fry):** Unix precondition/CAS hardening. Real `check_fs_precondition()` helper with self-heal; separate no-side-effect pre-sentinel variant for write path to preserve sentinel-failure truth. Scope: 12.2 + 12.4aa–12.4d.
- **Inbox decisions merged:** Bender M1b-i proof closure + Fry M1b-ii precondition split decision. Both now in canonical `decisions.md`.
- **Status:** Awaiting final Professor + Nibbler gate approval for both M1b-i and M1b-ii before landing.

## 2026-04-25 — Slice 13.6 + 17.5ddd Review (Bender revision)

**VERDICT: REJECT**

The implementation in `src/core/vault_sync.rs::parse_ignore_parse_errors()` silently strips every `file_stably_absent_but_clear_not_confirmed` entry, retaining only `code == "parse_error"` entries. `design.md §505` — the single authoritative schema document referenced by both task 13.6 ("returns the per-collection object documented in design.md") and 17.5ddd ("response shape matches design.md schema exactly") — explicitly states the `ignore_parse_errors` field covers **both** `"parse_error"` (line-level glob failure) **and** `"file_stably_absent_but_clear_not_confirmed"` (stateful-absence refusal). `design.md` was not modified in this diff. The test `brain_collections_surfaces_status_flags_and_terminal_precedence` (in `src/mcp/server.rs`) cements the violation: it seeds a `file_stably_absent_but_clear_not_confirmed` row and asserts `absent["ignore_parse_errors"].is_null()`, which is directly contrary to what the spec says should be surfaced.

All four of Bender's other claimed fixes are correct and well-covered: `integrity_blocked` precedence is right, the 30-minute escalation default and `GBRAIN_MANIFEST_INCOMPLETE_ESCALATION_SECS` env-var are correct, `restore_in_progress` semantics match the spec, and `recovery_in_progress` queued-vs-running split is properly tested.

**Minimum required fix:** Either (a) update `design.md §505` to explicitly exclude `file_stably_absent_but_clear_not_confirmed` from `brain_collections` output and document the deferral, or (b) remove the `retain(|e| e.code == "parse_error")` filter and surface both codes as the spec demands. The test must be updated to match whichever path is chosen.




## 2026-04-24 — 13.5 re-review after repair commit 97e574e

VERDICT: APPROVE

The repair is tight and correct. progressive_retrieve now accepts collection_filter: Option<i64> and outbound_neighbours enforces it via AND (?3 IS NULL OR p2.collection_id = ?3) — the ?3 IS NULL short-circuit keeps the CLI path (which passes None) unaffected. rain_query in server.rs threads the same collection_filter ID it resolved for hybrid_search_canonical straight into progressive_retrieve, so the initial search and the BFS expansion are under the same fence. The CLI query.rs correctly passes None (no widening). The esolve_read_collection_filter default logic satisfies the 13.5 contract — single active collection filters to it, write-target designated filters to it, no write target returns None (all collections). The new direct-proof test rain_query_auto_depth_does_not_expand_across_collections creates an explicit cross-collection link and asserts the linked page in the foreign collection never surfaces. Scope is clean: no write-path changes, no CLI widening, no ignore-diagnostic widening. 13.5 is sealed.
- **Pre-gate scoping before implementation (2026-04-25):** A pre-gate decision on narrow restore re-enable must specify the exact contract before any code lands: which fsyncs are mandatory, what the observable invariant is at recovery time, and which failure paths are acceptable. The contract in this case was: parent fsync after every unlink, install-time no-replace semantics, and deterministic no-data-left-behind on any failure. Gating before the body starts prevents discovering the contract mismatch in code review.
