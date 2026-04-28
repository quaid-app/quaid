# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- **PR #110 final CI compile fix — PathBuf vs &str in match guard (2026-05-XX):** A `WatchEvent::DirtyPath(path) if path == "notes/already-buffered.md"` guard fails to compile because `PathBuf` does not implement `PartialEq<&str>`. The minimum correct fix is `path == PathBuf::from("notes/already-buffered.md")`, consistent with every other `DirtyPath` guard in `vault_sync.rs`. Do not use `.to_str()` or `.to_string_lossy()` here — those introduce UTF-8/OsStr indirection that the existing codebase pattern avoids. Check the local pattern before choosing a comparison form.

- **Batch 1 scope narrowing— direct artifact repair post-professor-rejection (2026-04-28):** When Professor gates a batch due to three intertwined contradictions (authorization bypass + frozen schema widening + unreachable enum variants), the most efficient repair is direct artifact editing rather than iterating back to the design document. Narrowing: move overflow-recovery mode to `FullHashReconcileMode`, keep authorization as `ActiveLease`; CLI-only watcher health on `quaid collection info` (not MCP widening); `WatcherMode` as `Native | Poll | Crashed` only. Update implementation_plan.md + tasks.md + decisions inbox. Enforce implementer lockout (Fry → Mom). Result: Batch 1 scope now honestly closable under narrowed v0.10.0. Orchestration logs for professor gate + leela repair both recorded.
- **PR review routing — docs-truth-resolution vs. code-gate-removal (2026-04-25):** When a PR reviewer flags a docs-code mismatch, the resolution can go either direction: fix the code to match the docs, OR fix the docs to match the code. The correct direction is the product decision, not the technical analysis. For PR #77 the reviewers argued the Unix gate on `gbrain serve` was over-engineering; Amy's product decision was to keep the gate and make the docs accurate. The correct routing call was: accept Amy's decision, commit the accurate docs, leave the gate in place. A pure technical reading of `start_serve_runtime` being cross-platform missed the product intent. Always check the product owner's final commit message and any in-progress docs before implementing a code fix from review comments.

- **PR review thread deduplication (2026-04-25):** Multiple review threads about the same root cause (e.g. all `brain_collections` schema threads, all `gbrain serve` platform-gate threads) must be grouped before implementation. Count unique root causes, not thread count. For PR #77 there were ~23 threads but only 4 root causes. Treat the fix set as 4 items, not 23.

- **M1b repair — tasks.md closure notes are truth claims (2026-05-XX):**A tasks.md closure note that says "explicit mutator matrix in `src/mcp/server.rs`" is a truth claim about WHERE production behavior lives. If the actual gates are in command-layer library functions (`commands/link.rs`, `commands/check.rs`), the note is false regardless of whether the tests pass. When closing a task, the closure note must name the actual file and function that enforces the behavior — not just the entry-point that calls it. Proof-only tasks must be verified proof-only: no production gates may silently appear in callee functions under a "tests-only" claim.

- **M1b-ii — collection interlock ordering rule (2026-05-XX):** Any MCP handler that does both collection state checking and OCC/precondition checking MUST run the collection state check (`ensure_collection_write_allowed`) BEFORE the OCC check. If the OCC check runs first on a blocked collection, version-conflict or existence-conflict errors leak through instead of `CollectionRestoringError`. The fix is a single `ensure_collection_write_allowed` call immediately after `resolve_slug_for_op`. This is cross-platform (pure DB state check) and requires no `#[cfg(unix)]` gate. Add ordering-proof tests that put the collection in restoring state with a pre-existing page (for the "already exists" case) and with a ghost expected_version (for the "does not exist at version N" case) — both must return `-32002 CollectionRestoringError`, not `-32009`.

- **Post-K2 batch routing — code-in-tree vs. proof-missing asymmetry (2026-04-23):** When RCRT code is already committedand its startup integration tasks are checked but the proof tests are open, the next batch is proof closure, not new surface. The pattern is: (1) identify the last deferred "proof" cluster that pairs with completed code; (2) add the structural initialization tasks that the proof depends on (`11.1` registries before `11.5` RCRT); (3) close the integration test that exercises the full crash-to-recovery scenario. Never open IPC or `brain_put` write-through while the orphan recovery proof is outstanding — the proof gap creates a false-confidence floor that contaminates every subsequent batch's safety claim.

- **Batch J rescope — reviewer disagreement resolution (2026-04-23):** When one reviewer approves a batch only as an inseparable combined slice and another reviewer rejects the same batch as too broad, the resolution is not to split arbitrarily — it is to find the smallest coherent slice that (a) lands the one real new operator behavior, (b) carries only the proofs necessary to keep that behavior honest, and (c) defers anything that requires fresh code paths under a "proof" label. The deferred items become the next batch's explicit scope. Both reviewers must re-confirm the rescoped boundary before implementation; an approving reviewer's earlier approval does not automatically carry over when the scope changes.

- **Batch K scoping after a CLI-truth-only batch (2026-04-23):** When a batch narrows to CLI truth only and explicitly defers "finalize/handshake/integrity matrix closure beyond the CLI truth surface," the correct follow-on is to close that deferred matrix — not to open adjacent breadth (watcher, brain_put, IPC). The proof work belongs in the next batch because: (1) the library code is already in the tree and has been approved, so deferring proofs further increases the gap between implementation and verification; (2) the CLI truth seam from the prior batch is only fully proven once the deep integration test (17.11 offline restore end-to-end) exists; (3) the operator entry point (`collection add`) needed for integration test scaffolding is a natural co-traveler that costs less code than fabricating DB fixtures.

- **Batch J repair — fail-close invariant for `--finalize-pending` (2026-04-23):** The CLI truth layer for `gbrain collection sync <name> --finalize-pending` must not emit success-shaped output for non-final `FinalizeOutcome` variants. The original implementation unconditionally called `render_success` for all outcomes, including `Deferred`, `ManifestIncomplete`, `IntegrityFailed`, `Aborted`, and `NoPendingWork`. Fix: match on `FinalizeOutcome`; only `Finalized` and `OrphanRecovered` are success; every blocked outcome bails with `FinalizePendingBlockedError: collection=… outcome=… collection remains blocked and was not finalized`. Rule: any branching code path in a CLI command that feeds into automation must be fail-closed — if a non-success semantic outcome exists, the only safe default is to make it visible as a non-zero exit. Do not centralize success rendering above a match; the match is the rendering decision.

 but the default operator entrypoint and recovery proofs are still open, the next batch should finish that same seam before opening adjacent breadth. For vault-sync after Batch I, plain `gbrain collection sync <name>` plus restore/remap/RCRT integrity proof is the right follow-on; IPC writes, watcher breadth, and wider collection UX can wait.

- **Next-batch routing after a library-first safety slice (2026-04-22):** When a destructive workflow is intentionally split into library-first phases and later command wiring, the next batch should finish that same workflow end-to-end before starting adjacent surfaces like watcher or `brain_put`. For vault-sync, once Batch H landed Phase 0–3 restore/remap safety helpers plus fresh-attach, the coherent follow-on is restore/remap orchestration (Phase 4 + CLI/serve ownership/recovery), because it reuses the new authorization seam immediately and keeps the highest-risk state machine reviewable in one place.

- **Batch H auth repair (2026-04-22):** Drift-capture/full-hash bypasses must be keyed to persisted owner identity, not just enum class + non-empty token. The narrow fail-closed shape for this seam is to persist `active_lease_session_id`, `restore_command_id`, and `restore_lease_session_id` on `collections`, compare exact equality before opening the root, and reject when the persisted owner is missing or mismatched. Keep fresh-attach on its own command-only mode so attach does not inherit the restore/remap bypass surface.

- **Batch H scoping (2026-04-22):** After Batch G, `full_hash_reconcile` is real and the apply pipeline is live, directly unblocking the restore/remap safety pipeline. Batch H = restore/remap Phase 0–3 preflight + Phase 1–3 library core: 5.8a0 (UUID-migration preflight), 5.8a (RO-mount gate), 5.8a2 (dirty-preflight), 5.8b (Phase 1 drift capture), 5.8c (Phase 2 stability), 5.8d (Phase 3 fence), 5.8d2 (TOCTOU recheck), 5.9 (fresh-attach wiring), plus `--allow-rerender` CLI flag (deferred from 5.4h) and tests 17.5aaa, 17.5ii2-3, 17.5ii6-7, 17.5ii9a, 17.5ccc. Gate: all safety pipeline functions exist as library-level code, are unit-tested, and provably prevent the destructive step from running without all preceding phases passing. Deferred: Phase 4 bijection (5.8e), online/offline CLI execution (5.8f/g), UUID writeback (5a.5+), Groups 6/9/11/12. Two highest-risk seams: (1) 5.8b bypass authorization contract — `full_hash_reconcile` called in `synchronous_drift_capture` mode bypassing `state='active'` gate; Professor must sign off on mode/lease signature BEFORE Fry starts the body; (2) 5.8a0 trivial-content threshold — must call the SAME predicate/helper as the 5.3 rename guard (body ≤ 64 bytes after frontmatter); Nibbler must verify predicate consistency. Tasks.md wording changes required: 5.4h (--allow-rerender Batch H note), 5.8 container (Batch H scope boundary), 5.8a0 (17.5ii9a stub note), 5.8b (Professor pre-gate required), 17.5aaa (Batch H target note).

- **Restore/remap safety pipeline is library-first, CLI-second (Batch H):** The entire Phase 0–3 pipeline (preflight checks + drift capture + stability + fence + TOCTOU) can be implemented as pure library functions before the CLI restore command (9.7) or serve integration (11.*) exist. Each function takes a `collection_id` + lease/authorization context and returns a typed result — no clap wiring required. This means Fry can implement and test the safety core in one coherent batch, and Group 9/11 can later wire it end-to-end without changing the library contract. Rule: always ask whether a complex pipeline's library core can be isolated from its CLI entry point; if yes, implement and gate the library first.

- **Batch G repair — apply_reingest zero-total raw_imports guard (2026-04-22):**`rotate_active_raw_import()` correctly allows `row_count == 0` for genuine new-page bootstrapping. But `apply_reingest` on the stat-diff path calls it without first verifying whether the page is new or existing. For existing pages, `row_count == 0` is corrupt state, not bootstrap state. The fix is a pre-flight guard in `apply_reingest` — after `load_existing_page_identity` resolves the page, before any mutation — that returns `InvariantViolationError` if the existing page has zero total raw_imports rows. The guard covers both the explicit `existing_page_id = Some(...)` path (modified files) and the `existing_page_id = None` slug-match path (file appears at new path with slug matching existing DB page). Two adversarial `#[cfg(unix)]` regression tests added. Rule: when a helper function has a dual personality (bootstrap-OK for new pages, error for existing pages), the caller — not the helper — is the right place to enforce the existing-page precondition, because only the caller has the context to distinguish the two cases.

- **Batch G scoping (2026-04-22):** After Batch F, the apply pipeline is live and the reconciler can do real mutations — but `full_hash_reconcile` still returns `Err("not yet implemented")`, blocking restore/remap (5.8b), watcher overflow recovery (6.7a), fresh-attach (5.9), and RCRT (9.7d). Batch G closes this with 4.4 (full_hash_reconcile real impl) + 5.4h (InvariantViolationError guard wired into full_hash_reconcile, restore guard deferred to Batch H) + 5a.6 (render_page always emits gbrain_id for non-null uuid) + 5a.7 partial (gbrain_id round-trip, UUIDv7 monotonicity, brain_put gbrain_id preservation). Gate: full_hash_reconcile is callable, correct, idempotent, and guarded; brain_put round-trips preserve page identity. Highest-risk seam: full_hash_reconcile raw_imports hash-match-vs-rotation logic — unchanged content must update stat fields only (no rotation), changed content must rotate atomically, zero active rows must fire InvariantViolationError. Nibbler adversarial review required. Professor must gate the full_hash_reconcile function signature (mode param / lease authorization design) before Fry starts the body. Four tasks.md wording fixes required: 4.4 (Batch G target + mode param note), 5.4h (split: full_hash_reconcile hookup in Batch G, restore hookup in Batch H), 5a.6 (scope limited to render_page, NULL uuid must not emit field), 5a.7 (split partial/deferred). Deferred: 5.8* (Batch H, now unblocked by 4.4), 5.9 (needs Group 9 scaffold), 4.6 background task (needs Group 11), 5a.5+ (needs Group 12), Groups 6-12.
- **full_hash_reconcile is not just reconcile-without-stat-diff (Batch G):** The critical behavioral distinction is how it handles pages whose hash is UNCHANGED. Normal reconcile skips unchanged files (stat match → no re-hash). full_hash_reconcile hashes everything but must still detect "same hash → update stat fields only, no raw_imports rotation." Getting this wrong (rotating even on hash-match) silently produces duplicate active raw_imports rows, corrupting the restore anchor. The three cases must be explicit in the implementation: (1) hash unchanged → stat update only; (2) hash changed → full rotation; (3) zero active rows → InvariantViolationError. This is a data-loss surface on par with the Batch F raw_imports atomicity finding.

- **Batch F scoping (2026-04-22):** After Batch E, the reconciler can classify every file (unchanged/modified/new/missing/renamed/quarantined-ambiguous) but cannot mutate anything — every reconcile pass is still read-only. Batch F closes this gap with raw_imports rotation (5.4d/e/g/h) + apply pipeline (5.5/5.6/5.7) + quarantine lifecycle tests (17.5g–j) + raw_imports invariant tests (17.5xx–17.5aaa1). Gate: `gbrain collection sync` produces real DB mutations on first pass and zero on second pass; every write-path test asserts exactly one active raw_imports row per page. Three tasks.md wording fixes required before Batch F starts: 5.4d batch-tx boundary clarification, 5.5 Unix-only + re-evaluate-at-apply-time note, 4.4 explicit "Batch G" deferral note. Deferred: 5.4f (daily sweep needs serve), 4.4 + 5.8* (restore/remap Batch G), 5a.5+ (UUID write-back needs Group 12 first), Group 6 (watcher), Group 12 (brain_put rename-before-commit).
- **raw_imports rotation atomicity is a data-loss surface (Batch F):** The `is_active` flip (prior→0, new→1) and inline GC (5.4e) must be inside the same SQLite tx as the pages/file_state upsert. If split, a crash between transactions can leave a page with zero active raw_imports rows — making it permanently unrestorable via `gbrain collection restore`. The batch-of-500 commit (5.6) is the tx boundary; raw_imports rotation is per-file within each batch chunk, never outside it. Nibbler adversarial review required before merge, covering: (a) re-ingest produces exactly one active row; (b) GC cap never deletes the newly-inserted active row; (c) KEEP_ALL=1 bypasses GC without touching the active row; (d) simulated tx rollback after is_active=0 but before new row insert leaves the prior active row intact.
- **Quarantine-vs-hard-delete verdict at apply time (Batch F):** The apply pipeline (5.5) must re-evaluate `has_db_only_state` at apply time, not use the classification-time snapshot. Between classification and apply, a concurrent agent might remove the last programmatic link, changing the verdict from quarantine to hard-delete. Re-evaluating is the conservative, correct behavior. This must be explicit in the tasks.md 5.5 note so the implementer doesn't accidentally cache the snapshot.

- **Batch E repair — body-size vs whole-file-size in hash-rename guard (2026-04-22):** The ≥64-byte threshold in the conservative hash-rename guard (`hash_refusal_reason`) MUST apply to body bytes after frontmatter, not whole-file size. Using `file_state.size_bytes` (whole-file) allows large-frontmatter / tiny-body template notes to satisfy the threshold and be incorrectly paired as renames. Fix: replace `size_bytes` fields in `MissingPageIdentity` and `NewTreeIdentity` with `body_size_bytes` computed from parsed body content (not filesystem stat). This is consistent with spec language in tasks 5.8a0 and 5.8e which explicitly say "body size ≤ 64 bytes after frontmatter". Rule: any ≥64-byte content guard in a rename/identity context measures body bytes, never whole-file bytes.

- **Batch E scoping (2026-04-22):** After Batch D, the reconciler can walk + quarantine-classify but cannot resolve identity across rename events. The natural next slice is UUID lifecycle (5a.1–5a.4, 5a.4a) + rename resolution (4.5, 5.3, 5.3a) + tests (17.5b–f). This batch answers "what is the identity of every file in the walk?" without yet acting on the answer. The gate is crisp: all three rename-detection paths work (UUID, content-hash, quarantine-on-ambiguity); default ingest is read-only with respect to user bytes. The apply pipeline (5.5–5.7), raw_imports rotation (5.4d–g), and full_hash_reconcile (4.4) are correctly deferred to Batch F — they act on the classification Batch E produces. Three tasks.md wording fixes required before Batch E starts: 5.3 native-events scope note, 5a.3 construction-site cascade warning, 4.5 dependency on 5a.1–5a.4.
- **Page.uuid non-optional cascade risk (Batch E):** Making `uuid: String` non-optional on the `Page` struct will cascade to ~15+ construction sites (ingest.rs, migrate.rs, test fixtures, MCP response constructors). The failure mode is a silent default (empty string, placeholder constant) that passes the compiler but corrupts UUID-based rename matching. Professor must audit every construction site before merge; zero placeholder defaults are acceptable.
- **Content-hash uniqueness guards are a data-destruction surface (Batch E):** The three guards in 5.3 (≥64 bytes, unique hash in both missing and new sets, non-empty body after frontmatter) prevent false identity matching on templates and trivial notes. A bug here silently loses page identity permanently. Nibbler adversarial review is required (not optional) before merge, covering ambiguous, trivial-content, and guard-failure-logging edge cases.
- **Native event pairing interface vs. event source (Batch E):** Task 5.3 step (1) defines the function interface for watcher-provided (from, to) pairs, but the watcher (Group 6) does not land in this batch. The function must accept a `has_native_events` flag (or equivalent) that defaults to false so cold-start reconciler tests can exercise steps (2)–(4) only. This must be captured in a tasks.md note before implementation starts.

- **Batch D scoping (2026-04-22):** The walk (5.2) is the single largest unblock in the vault-sync engine — every downstream task (rename resolution, apply, full_hash_reconcile, quarantine classification) requires a real filesystem traversal. Group walk + quarantine predicate (5.4 series) as Batch D; they are logically independent of rename resolution (5.3) and apply (5.5) but together answer "can we walk a vault AND know what to do with each file?" The gate is crisp: reconciler idempotency + symlink rejection + five-branch predicate tests. Defer rename resolution, apply, raw_imports rotation, and restore/remap to later batches.

- **Batch D tasks.md truthfulness repair (2026-04-22):** Three stale/false notes caught by Professor gate: (1) task 4.3's "Foundation complete" note still claimed the real walk was deferred to 5.2 even after 5.2 landed — always update upstream task notes when a downstream task closes the gap; (2) task 5.1's Batch C repair note still named `walk_collection` and `has_db_only_state` as `Err`-returning stubs after Batch D made them real — multi-batch notes need addendum lines rather than in-place rewrites so audit trails survive; (3) task 5.4a claimed `extract_links()` sets `wiki_link` but `extract_links()` only returns `Vec<String>` and never writes to the DB — always verify function signatures, not just intent, before claiming a callsite populates a DB column. Rule: a task note is a truth claim about the current tree, not a description of intent.
- **WalkBuilder symlink risk:** `ignore::WalkBuilder`'s `follow_links(false)` is NOT equivalent to `AT_SYMLINK_NOFOLLOW`. Per-entry `stat_at_nofollow` must be called explicitly inside the readdir loop to get fd-relative NOFOLLOW semantics. This is the highest-risk seam in Batch D and warrants Nibbler eyeball alongside standard reviewer coverage.
- **source_kind audit scope risk:** `has_db_only_state` depends on `source_kind` being correct at every `INSERT INTO links` callsite. A missed callsite silently corrupts the predicate, turning a quarantine-required page into a hard-delete candidate. Audits of this kind should include a recommendation to make source_kind non-defaultable at the schema level to prevent silent regressions.

- **Batch B repair (2026-04-22):** Safety-critical stubs must fail explicitly.`has_db_only_state` returning `Ok(false)` is worse than returning an error — it grants a "safe to delete" verdict for every page silently. Prefer `Err("not yet implemented")` over any success-shaped default on a predicate that gates data destruction. This is the Rust-best-practices "explicit error behavior, no success-shaped stubs for safety paths" rule in concrete form.
- **Framing discipline:** "replaces X" in a module header comment creates false expectations if X is still live. The honest framing is "will replace X once tasks N–M land". Review every new-module header comment for present-tense claims that aren't yet true before gate approval.
- Third-author gate verification: when Professor claims "no-side-effects" on legacy refusal, verify by checking that v5 DDL tables are absent post-rejection, not just that an error was returned.
- Gate approval requires `cargo test` + `cargo clippy -- -D warnings` both clean; `cargo fmt` is implicitly validated when clippy passes.
- After a foundation approval, route Groups 3–5 (ignore patterns, file state, reconciler) as the next implementation batch; 2.4a (`rustix` dep) must arrive with or before 4.2 since `stat_file` needs `fstatat`.
- `docs\spec.md` is the primary product spec.
- For a breaking schema change, the schema DDL update + test fixture update must be atomic.
- When a spec replaces an entire ingest path (import.rs → reconciler.rs), the new path must be complete before removal.
- IPC security surfaces need adversarial review (Nibbler) before implementation, not after.
- When a foundation slice is rejected with 181 test failures, a focused repair pass (not a full rewrite) can fix all blockers in one coordinated cycle if: legacy defaults are prioritized, schema compatibility shims are kept, and all write paths (upsert/filter) are wired together.
- Batch gate claims "test suites passed" but the gate also requires `cargo clippy -- -D warnings` clean. New scaffolding modules (not yet wired to commands) must include `#![allow(dead_code)]` — the same pattern reconciler.rs uses — or clippy will reject the build with unused-item errors. Verify both independently; don't conflate test-pass with gate-pass.
- When a scaffold batch honestly admits stubs (reconciler returns empty stats, has_db_only_state returns false), APPROVE if: the stubs are clearly marked with comments citing the task IDs for full implementation, no live code path calls the stub in a way that silently degrades behavior, and the contract types are correct. False-positive quarantine suppression from a non-functional reconciler is not a risk until the reconciler walk is wired.
- **Batch C gate (2026-04-22):** `cargo test` and `cargo clippy` passing on Windows does not guarantee Unix compilation. `#[cfg(unix)]` blocks are skipped entirely on Windows. When a task note claims "Unix path uses X", verify that X is actually imported under `#[cfg(unix)]` — missing conditional imports are invisible to Windows CI and will cause hard compile errors on Linux/macOS. This is a new class of overstatement: code that references the right symbols but doesn't compile on target.
- **Doc comment discipline for platform-split functions:** When a function has `#[cfg(unix)]` and `#[cfg(not(unix))]` variants, the public doc must describe what the ACTUAL function body does, not what a hypothetical future version might do. A doc that says "prefers fd-relative fstatat when parent_fd is provided" on a function whose signature is `fn f(path: &Path)` is an overstatement regardless of intent. Also: `lstat` does NOT follow symlinks; `stat` does. Do not confuse them in comments.
- **Batch C repair (2026-04-22):** The "success-shaped stub" anti-pattern applies beyond predicates — stub functions that return `Ok(ReconcileStats::default())` are equally dangerous on safety-critical recovery paths (restore, remap, audit). Any function called by a restore or remap path that returns zeroed success stats silently turns a non-existent reconciliation into an apparent clean pass. Extend the explicit-error rule: if a function is on a restore/remap/audit call path and is not yet implemented, it returns `Err`, not `Ok(empty)`.
- **Conditional imports for `#[cfg(unix)]` blocks must be declared at module level with matching `#[cfg(unix)]` guards.** Rustix is a Unix-only dep (`[target.'cfg(unix)'.dependencies]`); any use of its types (e.g. `OwnedFd`) in function signatures inside `#[cfg(unix)]` blocks requires a matching `#[cfg(unix)] use rustix::fd::OwnedFd;` at the top. Windows CI will silently skip these blocks; missing imports only surface on Linux/macOS cross-compilation.

## 2026-04-22 Vault Sync Foundation Repair Pass

**Session:** Leela integration-focused repair after Professor+Scruffy+Nibbler foundation rejections.

**What happened:**
- Vault-sync foundation slice was rejected by Professor for schema coherence (181 test failures) and incomplete task marking.
- Root cause: `NOT NULL` constraints on `pages.collection_id` and `pages.uuid` added without updating 20+ legacy INSERT sites and all filter paths.
- Leela took ownership of repair (Fry locked out under reviewer rejection protocol).

**Five decisions made (now canonical in decisions.md):**
1. `pages.collection_id DEFAULT 1` + auto-created default collection at `open_connection()`
2. `pages.uuid` becomes nullable (`DEFAULT NULL`) until UUID lifecycle tasks (5a.1–5a.7) are wired
3. `ingest_log` table retained as compatibility shim (removed only when watcher/reconciler fully replaces import)
4. Updated all `ON CONFLICT` clauses from `(slug)` to `(collection_id, slug)` across ingest.rs and migrate.rs
5. Added `AND p.quarantined_at IS NULL` filter to `search_vec` in inference.rs

**Outcome:**
- `cargo test`: 181 failures → **0 failures**
- All legacy write helpers now work with v5 schema without modification
- Default collection auto-created on every `open_connection()`
- Quarantine filtering wired consistently across FTS5 and vector search

**Learnings for future repairs:**
- Breaking schema changes can be made compatible by adding defaults and shims rather than rewriting all callers
- Consistency across read paths (FTS5 + vector search) must be verified in parallel, not sequentially
- Legacy support code (ingest_log shim) should be kept until its replacement is fully wired, not removed preemptively

**Status:** Foundation repair complete. 181 test failures resolved. Follow-on implementation batches now unblocked.

## Vault Sync Engine Breakdown — 2026-04-22

**Session:** macro88 directed team to treat `openspec\changes\vault-sync-engine` as next major enhancement with >90% test coverage.

**What was analyzed:**
- Full `openspec/changes/vault-sync-engine` spec: proposal.md, design.md, tasks.md (370+ tasks, 18 groups), 3 sub-specs, current v4 schema.

**Architecture decisions:**
- v4→v5 is breaking schema change. Every test, every page-touching module, entire ingest path affected from first commit.
- Keep as ONE OpenSpec change, implement in 3 gated PRs: Foundation (Waves 1–2) → Live Engine (Waves 3–5) → Full Surface (Waves 6–7, 9).
- Critical path: Schema → Collections → Reconciler → Watcher+brain_put → Commands/Serve → MCP.
- Highest-risk: two-phase restore/remap defense (task 5.8), brain_put crash-safety/IPC security (task 12.6), watcher overflow constraint (task 6.7a).

**First execution batch (PR A foundation):** Tasks 1.1–1.6, 2.1–2.6, 2.4a–d, 3.1–3.7, 4.1–4.4, 5a.1–5a.4a, 17.1–17.4. Scope: ~1 week, Fry owns. Does NOT touch watcher, reconciler, brain_put, MCP handlers.

**10 open questions with recommendations:** branch strategy (fresh feature branches), active in-flight work (resolve v0.9.3/v0.9.4 first), Windows CI gate, Nibbler IPC pre-review, raw_imports audit, macOS CI, Cargo.toml deps, import removal lint, coverage hard gate `cargo llvm-cov --fail-under-lines 90`, user v4 migration messaging.

**Team routing:** Nibbler reviews IPC security (12.6c–g) before Wave 5. Bender + Scruffy track 90%+ coverage every PR. Resolve 10 questions before/during Wave 1.

**Artifact:** `.squad/decisions/inbox/leela-vault-sync-breakdown.md` (305 lines, complete execution roadmap)

## Core Context

**Sprint 0 Foundation (2026-04-13):** Leela created 4 OpenSpec proposals (`sprint-0-repo-scaffold`, `p1-core-storage-cli`, `p2-intelligence-layer`, `p3-polish-benchmarks`) and full repo scaffold (24 CLI commands, 15 core modules, MCP stub, full schema DDL, 8 skill stubs, GitHub Actions CI/release workflows). Four sequential phases with hard gates: Phase 1 gate = round-trip test + MCP + static binary. Architecture: Fry owns implementation; Professor + Nibbler gate approval. Constraints: no pwsh.exe on machine; manual git/PR required.

**Phase 1 OpenSpec Unblock (2026-04-14):** Created all missing OpenSpec artifacts (design.md, 6 capability specs, tasks.md with 57 tasks in 12 groups). Architecture decisions locked: single rusqlite conn + WAL for concurrency, lazy Candle init via OnceLock, offline model weights (include_bytes), hybrid search (SMS shortcut → FTS5+vec → RRF merge), OCC with `-32009` error code, wing-level palace (room deferred to Phase 2), error split (thiserror in core, anyhow in commands).

**Links & Tags Contracts (2026-04-14):** Clarified two gate-blocking contracts: (1) Links use integer IDs in DB, slugs in app layer — resolver in db layer on insert/read. (2) Tags live exclusively in tags table (no OCC, idempotent via INSERT OR IGNORE, no page version bump). Unblocked Fry T10 and T11 implementation.

---

## 2026-04-14 Search/Embed/Query Revision — T14/T18/T19 Honesty Pass

**What was done:**
- Professor rejected Fry's T14–T19 artifact; Fry locked out of revision cycle.
- Root cause: inference.rs used a SHA-256 hash shim but was presented as BGE-small-en-v1.5.
  T18 and T19 were closed as done without acknowledging the quality gap from T14 incompleteness.
  The promised decision note in inbox was never written.
- Fixes applied:
  1. `src/core/inference.rs`: module-level PLACEHOLDER CONTRACT doc block added; `embed()` and
     `EmbeddingModel` docs clarified to name the hash shim explicitly.
  2. `src/commands/embed.rs`: runtime `eprintln!` warning added to `run()` so callers on stderr
     see that embeddings are hash-indexed, not semantic. Comment tells the next engineer when to remove it.
  3. `openspec/changes/p1-core-storage-cli/tasks.md`:
     - T14: `[~]` step broken into `[x]` EmptyInput guard + `[ ]` Candle forward-pass, with a
       BLOCKER note listing the exact missing assets and wiring steps.
     - T18: honest status note added — plumbing done, hash-indexed, runtime warning in place.
     - T19: honest status note added — plumbing done, FTS5 ranking unaffected, vector quality gap stated.
  4. `.squad/decisions/inbox/leela-search-revision.md`: full decision note written.
- Validation: `cargo test` 115/115 passed. `cargo check` clean.

**Key lessons:**
- When a task is `[x]` but its dependency is `[~]`, the honest answer is to add a caveat note,
  not to let the `[x]` stand silently. The reviewers will catch it.
- The model name in the DB (`bge-small-en-v1.5`) is the intended name for the real model, not a lie —
  but it creates a false impression when the implementation is a hash shim. The fix is documentation,
  not changing the DB seed.
- A promised decision note that isn't written is a review blocker in itself. Always write the note
  before closing the task.
- `eprintln!` to stderr is the right channel for runtime placeholder warnings: stdout stays parseable,
  tests don't capture stderr, and the warning can be found by grepping the run output.

**Decision file:** `.squad/decisions/inbox/leela-search-revision.md`

## 2026-04-14T04:56:03Z Revision Cycle Completion

- **Mandate:** Revise T14–T19 after Professor rejection. Address semantic contract drift, embed CLI ambiguity, placeholder truthfulness. Fry locked out; Leela takes over independently.
- **Outcome: APPROVED FOR LANDING** with 5 key decisions:
  - **D1:** Explicit placeholder contract in `inference.rs` module docs
  - **D2:** Runtime stderr warning on every `gbrain embed` invocation
  - **D3:** T14 blocker sub-bullets (explicit missing assets)
  - **D4:** T18 honest status note (plumbing ✅, hash-indexed until T14)
  - **D5:** T19 honest status note (plumbing ✅, similarity metric until T14)
- **No code logic changes:** T16–T19 plumbing untouched; public API stable.
- **Test validation:** 115 pass unmodified; stderr warnings not captured by harness.
- **Outcome:** Phase 1 search/embed/query lane ready for Phase 1 ship gate. Users see honest status; downstream planners see exact blocker list.
- **Orchestration log written:** `2026-04-14T04-56-03Z-leela-accepted-revision.md`
- **Decision merged:** `leela-search-revision.md` (5 decisions, 0 conflicts) → canonical `decisions.md`

## Phase 1 OpenSpec Unblock — 2026-04-14

**What was done:**
- Created all missing OpenSpec artifacts for `p1-core-storage-cli` to make `openspec apply` ready
- Verified: `openspec status --change "p1-core-storage-cli" --json` shows `isComplete: true`, all 4 artifacts `done`
- Artifacts created: `design.md`, `specs/core-storage/spec.md`, `specs/crud-commands/spec.md`, `specs/search/spec.md`, `specs/embeddings/spec.md`, `specs/ingest-export/spec.md`, `specs/mcp-server/spec.md`, `tasks.md`
- 57 actionable tasks in 12 groups; Fry executes on branch `phase1/p1-core-storage-cli`

**Architecture decisions locked:**
- Single rusqlite connection per invocation (no pool); WAL handles concurrent readers at OS level
- Candle model init via `OnceLock` — lazy, one-time per process; CPU-only in Phase 1
- Model weights: `include_bytes!` default (offline), `online-model` feature flag for smaller builds
- Hybrid search: SMS exact-match short-circuit → FTS5+vec fan-out → set-union merge (RRF switchable via config table)
- OCC: CLI exit code 1 + MCP JSON-RPC error `-32009` with `current_version` in error data
- Room-level palace filtering deferred to Phase 2; wing-only in Phase 1
- Error handling: `thiserror` in `src/core/`, `anyhow` in `src/commands/`
- MCP error codes: `-32001` not found, `-32002` parse error, `-32003` db error, `-32009` OCC conflict

**Key file paths:**
- Design: `openspec/changes/p1-core-storage-cli/design.md`
- Specs: `openspec/changes/p1-core-storage-cli/specs/*/spec.md` (6 files)
- Tasks: `openspec/changes/p1-core-storage-cli/tasks.md`
- Decision log: `.squad/decisions/inbox/leela-p1-openspec-unblock.md`

**Phase 1 scope boundary:**
- In: CRUD, FTS5, candle embeddings, hybrid search, import/export, ingest, 5 MCP tools, static binary
- Out (Phase 2): graph, assertions, contradiction detection, progressive retrieval, room-level palace, full MCP write surface

**Patterns learned:**
- `openspec status --change "<name>" --json` is the canonical check for artifact readiness
- spec-driven schema requires: proposal → design → specs/**/*.md → tasks.md (in dependency order)

## Phase 3 Archive and Final Reconciliation — 2026-04-17

**What was done:**
- Conducted two archive passes on `p3-skills-benchmarks` and `p3-polish-benchmarks`
- First pass (leela-phase3-archive.md): Archived p3-polish-benchmarks; held p3-skills-benchmarks pending gates 8.2 and 8.4
- Second pass (leela-phase3-final-reconcile.md): Both gates closed; finalized p3-skills-benchmarks archive
- Updated all documentation (README, roadmap, roadmap.md on docs-site) to reflect "Phase 3 complete"
- Updated PR #31 body with final truth: both proposals archived, both gates passed, ready to merge and tag v1.0.0
- Cleaned up sprint-0 orphan active copy

**Key decisions:**
- Archive only when gates are genuinely closed (not before)
- Docs must reflect honest project state ("pending" → "complete" only after gates pass)
- Atomicity: both Phase 3 proposals archived in same commit with docs for revert consistency

**Outcome:** Phase 3 engineering and documentation complete. Both OpenSpec proposals in archive. PR #31 ready for merge + v1.0.0 tagging.

**Files filed:**
- `.squad/decisions/inbox/leela-phase3-archive.md` (first pass — gate hold rationale)
- `.squad/decisions/inbox/leela-phase3-final-reconcile.md` (final pass — both gates closed, archive finalized)
- `openspec instructions <artifact-id> --change "<name>" --json` gives template + rules for each artifact
- Tasks must use `- [ ] N.M description` format or apply won't track them
- GitHub issues and OpenSpec both drive work intake.
- Meaningful changes require an OpenSpec proposal before implementation.

## 2026-04-16T14:59:20Z Simplified-install v0.9.0 Release — Leela Completion

- **Task:** Updated `.squad/identity/now.md` to reflect simplified-install / v0.9.0 shell-first focus
- **Changes:**
  1. Updated current sprint status and focus in `.squad/identity/now.md`
  2. Confirmed simplified-install as active phase
  3. Updated identity to reflect installation UX priority (shell-first approach)
- **Status:** ✅ COMPLETE. Team identity aligned with v0.9.0 release focus (shell-first, installer-centric).
- **Orchestration log:** `.squad/orchestration-log/2026-04-16T14-59-20Z-leela.md`

## 2026-04-14 Scribe Merge (2026-04-14T03:50:40Z)

- Orchestration logs written for Leela (Link contract review) and Fry (T02 db.rs completion).
- Session log recorded to `.squad/log/2026-04-14T03-50-40Z-phase1-db-slice.md`.
- Three inbox decisions merged into `decisions.md`:
  - Leela's Link contract clarification (slugs at app layer, IDs at DB layer, three type corrections)
  - Fry's db.rs decisions (sqlite-vec auto-extension, schema DDL, error types)
  - Bender's validation plan (anticipatory QA checklist)
- Inbox files deleted after merge.
- Fry, Leela, Bender histories updated with cross-team context.
- Ready for git commit.


## Phase 2 Kickoff — 2026-04-15

**What was done:**
- Phase 1 confirmed complete (v0.1.0 shipped, tagged on `main`).
- Created branch `phase2/p2-intelligence-layer` from `main`.
- Updated `.squad/identity/now.md` to Phase 2 focus.
- Wrote team execution split to `.squad/decisions/inbox/leela-phase2-kickoff.md`.
- Committed p0 OpenSpec archive (was untracked in `openspec/changes/archive/`).
- Opened PR `phase2/p2-intelligence-layer` → `main` (no-merge policy; owner reviews).
- Closed Phase 1 GitHub issues #2, #3, #4, #5.
- Updated Phase 2 issue #6 with branch + PR link.
- Created Phase 2 sub-issues for each agent lane.

**Team execution lanes:**
- Fry → Groups 1–9 (all implementation)
- Scruffy → 90%+ coverage, ≥200 tests
- Bender → integration + ship-gate scenarios
- Amy → project docs
- Hermes → website docs
- Professor → peer review gate (graph, progressive, OCC)
- Nibbler → adversarial review (MCP write surface)
- Mom → temporal edge cases

**Key architecture context for Phase 2:**
- All Phase 2 tables already exist in schema — NO DDL changes needed.
- OCC on `brain_put` is already done — do not re-implement.
- `src/core/novelty.rs` logic is complete; only plumbing into ingest needed (Group 6).
- `src/commands/link.rs` is fully implemented — Groups 9.1–9.3 delegate to it.
- MCP error code convention: `-32001` not found, `-32003` db error (established in Phase 1).
- Graph BFS must be iterative (not recursive) — D1 from design.md.
- Token budget from `config` table (key: `default_token_budget`), not hard-coded.

**Key file paths:**
- OpenSpec proposal: `openspec/changes/p2-intelligence-layer/proposal.md`
- Design decisions: `openspec/changes/p2-intelligence-layer/design.md`
- Task list: `openspec/changes/p2-intelligence-layer/tasks.md` (10 groups, 50+ tasks)
- Specs: `openspec/changes/p2-intelligence-layer/specs/*/spec.md`
- Decisions inbox: `.squad/decisions/inbox/leela-phase2-kickoff.md`

**Learnings:**
- When Phase N completes, immediately create the Phase N+1 branch from main — don't let it sit as untracked local state.
- GitHub issues for completed phases should be closed at kickoff of the next phase, not left open.
- OpenSpec archives are version-controlled artifacts — commit them to the active branch, not left untracked.

---

## Sprint 0 — 2026-04-13

**What was done:**
- Read full spec (`docs/spec.md`, 155KB, v4 spec-complete)
- Created 4 OpenSpec proposals: `sprint-0-repo-scaffold`, `p1-core-storage-cli`, `p2-intelligence-layer`, `p3-polish-benchmarks`
- Created full repository scaffold: `Cargo.toml`, `src/main.rs`, 24 command stubs, 15 core module stubs, MCP stub, `src/schema.sql` (full v4 DDL), 8 skill stubs, test fixtures, `benchmarks/README.md`, `CLAUDE.md`, `AGENTS.md`, `.github/workflows/ci.yml`, `.github/workflows/release.yml`
- Wrote decisions to `.squad/decisions/inbox/leela-sprint-zero.md`

**Key file paths:**
- Spec: `docs/spec.md`
- Schema: `src/schema.sql` (matches v4 DDL)
- CLI entry: `src/main.rs` (full clap dispatch)
- Commands: `src/commands/*.rs` (24 stubs)
- Core lib: `src/core/*.rs` (15 stubs)
- Skills: `skills/*/SKILL.md` (8 stubs)
- CI: `.github/workflows/ci.yml` and `release.yml`
- Proposals: `openspec/changes/*/proposal.md`

**Architecture decisions:**
- Four sequential phases with hard gates between them
- Phase 1 gate: round-trip test + MCP connects + static binary verified
- No Phase 2 until Phase 1 gate passes (enforced in proposal)
- Fry owns implementation; Professor + Nibbler gate each phase
- CI runs `cargo check` + `cargo test` + static binary verification on every PR
- Release workflow uses `cross` for musl static linking on Linux

**Constraints learned:**
- `pwsh.exe` (PowerShell 7) is NOT available on this machine. Use Python or Node to create directories.
- GitHub write tools are not available (cannot create issues or PRs programmatically). User must run git commands manually.
- The `create` tool requires parent directories to exist. Use a general-purpose agent with Python to create directory trees.

**Pending (needs human action):**
1. `git checkout -b sprint-0/scaffold && git add . && git commit -m "Sprint 0: scaffold" && git push`
2. Open PR to main
3. Create GitHub labels: `phase-1`, `phase-2`, `phase-3`, `squad`, `squad:fry`, `squad:bender`, etc.
4. Create GitHub issues for each phase/workstream (see `.squad/decisions/inbox/leela-sprint-zero.md`)

## 2026-04-14 T10 Contract Review — Tags Architecture Lock

**What was done:**
- Reviewed T10 tags command implementation contract before Fry's code landed
- Identified three-way conflict: schema + types + prior decisions all said `tags` table; tasks.md + spec scenario were stale drafts referencing defunct `pages.tags` JSON pattern
- Published contract decision: **tags live exclusively in `tags` table**
  - List: SELECT from tags table (no OCC)
  - Add: INSERT OR IGNORE (no OCC, idempotent)
  - Remove: DELETE (no OCC, silent no-op on nonexistent)
  - No page version bump on tag operations
- Corrected gate-blocking artifacts:
  1. `tasks.md` T10: three bullet points updated to reference `tags` table, removed stale OCC/re-put language
  2. `specs/crud-commands/spec.md` Add tag scenario: clarified "inserted into tags table; page row not updated"
- Decision note written to `.squad/decisions/inbox/leela-tags-contract-review.md`
- Impact: Unblocked Fry's T10 implementation; tags now proceed on corrected contract with no page version bump

## 2026-04-14 Phase 1 CLI Expansion Merge — Session Complete

**Scribe snapshot (2026-04-14T04:21:54Z):**
- Orchestration logs created for Fry (T06–T12 completion: 86 tests passing) and Leela (T10 contract review findings)
- Session log recorded to `.squad/log/2026-04-14T04-21-54Z-phase1-cli-expansion.md`
- Five inbox decisions merged into canonical `decisions.md`:
  - Fry's T08 list + T09 stats (11 tests, dynamic SQL, pragma_database_list path resolution)
  - Fry's T06 put slice (OCC 3-path contract, SQLite timestamp, frontmatter defaults, 8 tests)
  - Fry's T11 link + T12 compact (slug-to-ID resolution, link-close UPDATE-first, 10 tests)
  - Fry's T10 tags (unified subcommand, tags table direct writes, no OCC, 8 tests)
  - Leela's T10 contract review (tags table exclusive, 3 operations locked, 2 artifact corrections applied)
- Inbox files deleted after merge
- Fry and Leela histories updated with cross-team context
- Ready for git commit

## 2026-04-14 Search/Embed/Query Tight Revision — Professor Blocker Resolution

**What was done:**
- Fry locked out of revision lane; Leela took the artifact directly.
- All three Professor rejection blockers assessed against current tree.
- Tests were already passing (115). Inference shim documented with eprintln warning by Fry — accepted as compliant deferral.
- Two remaining concrete gaps fixed in `src/commands/embed.rs`:
  1. Mutual-exclusion guard at function entry — (slug+all), (slug+stale), (all+stale) now error with "mutually exclusive".
  2. `--all` corrected: now applies `page_needs_refresh()` content_hash check (spec: "skip if unchanged"). Previous code force-re-embedded everything on --all.
  3. `--depth` in query: added `/// Phase 2: deferred` doc comment to clap arg.
- 4 new tests added; 119 total pass.
- Verdict: ACCEPTED FOR LANDING. Written to `.squad/decisions/inbox/leela-search-revision-tight.md`.

**Learning:** Mixed-mode CLI flag validation belongs at function entry, not threaded through downstream conditionals. When a spec sweep flag says "skip if unchanged", --all and --stale should behave identically on the skip check — the flag distinction is user-intent signal, not a behavioral fork.

## 2026-04-15 Phase 2 OpenSpec Package Completion

**What was done:**
- Assessed the complete current-state of the codebase against the P2 proposal.
- Created all four required OpenSpec artifacts for `p2-intelligence-layer`; `openspec status` now shows 4/4 complete.
- Artifacts created:
  1. `design.md` — 8 key design decisions, risk table, migration plan, open questions
  2. `specs/graph/spec.md` — N-hop BFS, temporal filtering, graph CLI
  3. `specs/assertions/spec.md` — triple extraction, contradiction detection, check CLI
  4. `specs/progressive-retrieval/spec.md` — token-budget gating, depth flag, palace room
  5. `specs/novelty-gaps/spec.md` — novelty wiring into ingest, knowledge gaps log/list/resolve
  6. `specs/mcp-phase2/spec.md` — 7 new MCP tools (brain_link, brain_link_close, brain_backlinks, brain_graph, brain_check, brain_timeline, brain_tags)
  7. `tasks.md` — 10 groups, 49 tasks, assigned to Fry on branch `phase2/p2-intelligence-layer`

**Key scope findings from codebase audit:**
- OCC on `brain_put` is ALREADY fully implemented (SG-6 fix). Excluded from P2 tasks.
- `src/commands/link.rs` is ALREADY fully implemented (create, close, backlinks, unlink + 12 tests). MCP wiring only needed.
- `src/core/novelty.rs` logic is complete but NOT wired into ingest — wiring is a Group 6 task.
- `src/core/palace.rs::derive_room` is a stub returning `""` — real implementation is a Group 7 task.
- Groups 1–4 (graph + assertions) are pure net-new implementation.
- Groups 5, 8 (progressive retrieval + gaps) are pure net-new implementation.

**Decision file:** `.squad/decisions/inbox/leela-p2-openspec.md`

**Patterns learned:**
- When a proposal says "Full MCP write surface", always audit what's already implemented vs. stub before scoping. Several P2 items (link.rs, OCC) were done in Phase 1 and needed removal from P2 scope.
- `openspec status` is the canonical check. 4/4 is the only acceptable state before handing to Fry.

## 2026-04-15 SG-6 Final Blockers — Direct Fix (Nibbler 2nd Rejection)

**What was done:**
- Fry locked out after two rejections on `src/mcp/server.rs`; Leela took the two remaining Nibbler SG-6 blockers directly.
- **Fix 1 — OCC create-path**: Added guard in `None =>` branch of `brain_put`. When `expected_version: Some(n)` is supplied for a non-existent page, returns `-32009` with `current_version: null`. Previously silently created at version 1. Added test: `brain_put_rejects_create_with_expected_version_when_page_does_not_exist`.
- **Fix 2 — Bounded result materialization**: Added `limit: usize` to `search_fts` (with SQL `LIMIT ?n`) and `hybrid_search` (passes limit to FTS + truncates merged result). Updated all callers: server.rs, commands/search.rs, commands/query.rs, all FTS/search tests. Handler-level `truncate` removed from server.rs (now redundant).
- `cargo clippy -- -D warnings` clean; 152 unit + 2 integration tests pass.
- Committed: `ba5fb20` — `fix(mcp): address Nibbler SG-6 final blockers — OCC create-path and result truncation`
- Decision artifact: `.squad/decisions/inbox/leela-sg6-final-fixes.md`
- SG-6 NOT marked done — requires Nibbler approval.

**Learning:** "Truncate after materialization" is never sufficient for resource exhaustion protection. The limit must be pushed into the DB query (SQL LIMIT) to prevent full scans on large corpora. Always trace the result cardinality back to the SQL layer, not just the handler layer.

## 2026-04-15 Task 5.3 Review — REJECTED (documentation-accuracy violations)

**What was done:**
- Reviewed task 5.3 against all four p3-polish-benchmarks spec files:
  - `specs/coverage-reporting/spec.md`
  - `specs/documentation-accuracy/spec.md`
  - `specs/docs-site/spec.md`
  - `specs/release-readiness/spec.md`
- Workflow implementation (ci.yml, docs.yml, release.yml): CLEAN. Coverage job, docs build/deploy split, release artifact matrix + checksum re-verification all match specs.
- RELEASE_CHECKLIST.md: CLEAN. All deferred channels named explicitly.
- README install/status copy: CLEAN. Phase 1 "In progress", deferred channels labeled.
- Docs site structure and nav (astro.config.mjs, index.mdx): CLEAN. Install, status, roadmap, contribution paths all surfaced.

**Two violations found — both in Amy's docs work:**
1. **Phase 1 status inconsistency:** README says "🔨 In progress"; `install.md` and `roadmap.md` say "Not started." Violates the shared-status requirement in documentation-accuracy spec.
2. **Stale coverage docs:** `install.md` says coverage is "planned as part of Phase 3 polish." But ci.yml has a live coverage job with lcov artifact, GITHUB_STEP_SUMMARY, and optional Codecov upload. Violates coverage-reporting spec requirement that docs must point to the supported coverage surface.

**Deferred scope check passed:** npm, Homebrew, curl-installer, and benchmarks are absent from all four surfaces. No scope creep.

**Verdict:** REJECTED. Task 5.3 not marked done. Amy to revise `install.md` (phase status + coverage section) and `roadmap.md` (phase status). No workflow or README changes needed.

**Decision file:** `.squad/decisions/inbox/leela-p3-review.md`

**Key lessons:**
- When implementation work (coverage CI) lands before or alongside doc work, the doc author must audit the workflow files — not just the README — before finalizing copy. Calling a live feature "planned" is a documentation-accuracy violation even if the doc was originally written before the feature.
- Status tables must be updated in all doc surfaces atomically. A single canonical status row written once and symlinked/imported would prevent drift. Until that pattern exists, reviewers must check every table independently.

## 2026-04-15 P3 Doc Fix — Rejected Artifacts Revision Pass (Amy locked out)

**What was done:**
- Revised `install.md` and `roadmap.md` after Amy's rejection on Phase 1 status mismatch and stale coverage docs.
- Fixed Phase 1 status in both docs-site pages to match README: "🔨 In progress".
- Rewrote `install.md` coverage section to describe the live CI surface: `cargo-llvm-cov`, `lcov.info` artifact, job summary, optional Codecov upload. Explicitly stated coverage is informational (not gating).
- Fixed `reference/spec.md` checksum documentation: corrected `.sha256` format description from "hex digest only" to "standard shasum output: `hash  filename`", removed `awk '{print $1}'` from pseudocode, updated upgrade skill staging to use `STAGING_DIR` + platform filename + `shasum --check` directly, updated quick-install snippet to match README pattern.
- README and workflow files left unchanged — they were already correct.
- Reviewer re-review gates (5.1 Kif, 5.2 Scruffy) not marked complete.
- Decision note written to `.squad/decisions/inbox/leela-p3-doc-fix.md`.

**Key lessons:**
- Doc authors must audit CI workflow files directly before calling any feature "planned." Calling a live CI job "planned" is a documentation-accuracy violation even when the doc predates the implementation.
- The `.sha256` format matters: `shasum -a 256 file > file.sha256` produces `hash  filename` format (two spaces). If you stage a binary to a different path than the artifact name in the `.sha256`, `--check` won't find the file. Solution: preserve the artifact filename in the staging directory so `--check` works directly.



**What was done:**
- Re-scoped `openspec/changes/p3-polish-benchmarks` away from an all-remaining-Phase-3 catch-all and toward the work that is actually ready now: release readiness, stale-doc fixes, free coverage on `main`, and docs-site polish.
- Updated the proposal frontmatter and body so the change now depends on `p1-core-storage-cli`, not `p2-intelligence-layer`, and names four concrete capabilities: `release-readiness`, `coverage-reporting`, `documentation-accuracy`, and `docs-site`.
- Created the missing apply-blocking artifacts: `design.md`, four capability specs, and `tasks.md` with explicit routing for Fry, Amy, Hermes, and Zapp.
- Wrote a decision note to `.squad/decisions/inbox/leela-p3-unblock.md` recording the scope cut: npm global distribution and simple installer UX stay documented as deferred follow-on work instead of being smuggled into this slice.

**Learning:**
- A phase proposal that tries to carry every remaining “someday” item becomes un-implementable. The fix is to cut to the smallest reviewable public surface that is truly ready now, then document the deferrals explicitly.
- Docs honesty needs an explicit supported-now / planned-later split. Otherwise README, website, and workflow polish drift independently and reviewers end up arguing about implied promises instead of concrete deliverables.

## 2026-04-15 P3 Release — Completion

**Role:** OpenSpec unblock architect, spec/scope conformance reviewer

**What happened:**
- Leela's P3 unblock proposal successfully narrowed `p3-polish-benchmarks` to ready-now scope: release readiness, README/docs fixes, coverage on `main`, and docs-site polish.
- Fry implemented coverage job (`cargo-llvm-cov` + standard checksum format), Zapp hardened release copy, Amy refreshed docs, Hermes improved docs-site UX.
- Kif's review (task 5.1) and Scruffy's review (task 5.2) both rejected twice on doc-drift issues. Both teams applied fixes and re-passed review gates.
- Final spec/scope conformance check completed and approved.

**Outcome:** P3 Release project **COMPLETE**. Coverage visible in GitHub UI, release workflow hardened, README/website/workflow docs all aligned, all gates passed. Project ready for release.

**Decision note:** `.squad/decisions.md` (merged from inbox) — P3 Release section documents all routing, decisions, gate feedback, and final approvals.

## 2026-04-15 Phase 2 Kickoff — Architecture Completion

**Role:** Phase 2 director, OpenSpec unblock architect, decision logger

**What happened:**
- Leela created complete OpenSpec artifact set for `p2-intelligence-layer`: design.md (8 design decisions), specs/graph/spec.md, specs/assertions/spec.md, specs/progressive-retrieval/spec.md, specs/novelty-gaps/spec.md, specs/mcp-phase2/spec.md, tasks.md (49 tasks across 10 groups).
- Defined scope boundary decisions: OCC on brain_put excluded (Phase 1), commands/link excluded (Phase 1), novelty logic excluded (Phase 1), derive_room included (real logic in Phase 2), graph BFS iterative not recursive, assertions regex not LLM, progressive depth 3-hop hard cap, room taxonomy freeform from heading.
- Established reviewer routing: Professor (Groups 1, 5, Task 10.6), Nibbler (Group 9, Task 10.7), Mom (temporal Task 10.8), Bender (ingest Task 10.9).
- Created branch `phase2/p2-intelligence-layer` from main at v0.1.0.
- Opened PR #22 (not merged per user directive — user reviews + merges).
- Updated issue #6 to in-progress; created 8 sub-issues per agent lane (Fry, Scruffy, Bender, Amy, Hermes, Professor, Nibbler, Mom).
- Committed Sprint 0 + Phase 1 OpenSpec archives to branch.

**Critical blockers identified (Professor + Nibbler + Bender):**
1. Schema gap: `knowledge_gaps.query_hash` missing UNIQUE constraint — blocks Group 8/9 validation
2. Graph contract ambiguity: undirected vs outbound-first — blocks Group 1 sign-off
3. Edge deduplication on cycles missing — blocks Group 1 sign-off
4. Progressive retrieval not started; contract unclear — blocks Group 5 sign-off
5. OCC erosion risk in Group 9 MCP writes — blocks Group 9 sign-off
6. Active temporal reads must check both interval ends — ship-gate blocker (Nibbler D1)
7. Graph traversal needs output budgets, not just hop cap — ship-gate blocker (Nibbler D2)

**Team coordination:**
- 6 agents completed planning (Leela kickoff, Scruffy coverage, Bender validation, Amy docs, Professor review, Nibbler guardrails)
- 2 agents running implementation (Fry Groups 1–9, Hermes website docs)
- 1 agent running edge-case review (Mom temporal links)
- All agents aligned on blockers and ready to work
- Orchestration logs written for each completed agent
- Session log recorded
- Decision inbox merged to decisions.md (14 items)

## 2026-04-17 P3 Archive Finalization

**What was done:**
- Reviewed uncommitted diff across all three `p3-polish-benchmarks` archive files. Changes were truthful and correct: `status: complete` → `status: shipped`, added `archived: 2026-04-17` frontmatter, Ship Gate section in tasks.md, and curly-quote normalization.
- Committed and pushed to `phase3/p3-skills-benchmarks`. Branch now clean and fully synced with origin; PR #31 reflects final state.

**Learning:**
- When a Scribe commit lands ahead of an archive update, always inspect the remaining diff before committing — the changes may be a mix of trivial normalization and meaningful metadata corrections, both worth keeping.
- Cross-agent history updates applied

**Outcome:** Phase 2 architecture **COMPLETE**. Blockers visible to all teams. PR #22 open and in review queue. Team can execute Phase 2 implementation with clear gates and parallel lanes.

**Decision notes:** `.squad/decisions.md` (merged from inbox) — Phase 2 Kickoff section documents all 6 leela decisions (D1–D6), full planning artifacts per agent, blocker findings from Professor and Nibbler, and guardrails for ship gate.

## Learnings — v0.9.1 Dual Release OpenSpec Cleanup (2026-04-19)

**Task:** Audit and normalize OpenSpec artifacts for the `bge-small-dual-release-channels` change after a session crash left the approved change with an empty tasks.md and a duplicate/obsolete change tree at `dual-release-distribution/`.

**What was done:**
- Audited both `bge-small-dual-release-channels/` (approved, has `.openspec.yaml`) and `dual-release-distribution/` (unapproved duplicate using old `slim` naming).
- Confirmed implementation is already on `release/v0.9.1-dual-release` (at main HEAD) using correct `airgapped`/`online` naming throughout: `install.sh`, `postinstall.js`, `release.yml` all verified.
- Wrote complete machine-parsable `tasks.md` for `bge-small-dual-release-channels/` with Phases A–D. A.1–C.3 marked done; D.1 (validation run) and D.2 (push + PR) remain open.
- Removed `openspec/changes/dual-release-distribution/` in full — it was unapproved, used stale `slim` naming, and had no `.openspec.yaml`.
- Updated `.squad/identity/now.md` to reflect v0.9.1 dual-release focus.
- Wrote decision record to `.squad/decisions/inbox/leela-dual-release-openspec.md`.

**Key lessons:**
- An empty tasks.md is indistinguishable from "no tasks" to the OpenSpec tooling. Always populate tasks.md before closing the artifact-creation step, even if implementation is proceeding in parallel.
- When two change trees exist for the same feature, confirm which has `.openspec.yaml` registration — that is the authoritative one. The other should be removed, not left to confuse future agents.
- Before marking implementation tasks done, check the actual code. In this case, the implementation correctly used approved `airgapped`/`online` naming — not the `slim` naming in the obsolete duplicate.
- Archive readiness requires only D.1 + D.2 to close; no additional OpenSpec artifact changes are needed.

**Archive gate note:** `bge-small-dual-release-channels` is ready to archive once D.1 (validation) and D.2 (PR) close. No proposal/design/spec changes needed.

## Learnings — v0.2.0 Release (2026-04-16)

**Task:** Create v0.2.0 GitHub release for Phase 2 — Intelligence Layer (PR #22 merged).

**Key decisions made:**

1. **Version bump method:** Edited `Cargo.toml` directly (0.1.0 → 0.2.0), ran `cargo check --quiet` to validate. Cargo.lock updated automatically. Did not do a full `cargo build` — version bump validation only.

2. **Release notes scope:** Wrote user-facing notes covering all 7 Phase 2 feature areas, new MCP tools (7 tools), new CLI commands (5), test milestone (533 tests, 90%+), and bug fixes from PR review. Based on Phase 2 OpenSpec proposal, tasks.md (58 completed tasks), and commit log.

3. **Release notes file lifecycle:** Wrote to `release-notes.md` at repo root, used it for `gh release create --notes-file`, then deleted it. Kept repo clean.

4. **Protected branch handling:** `git push origin main` succeeded despite branch protection bypass warning (remotes allowed it). Tag pushed separately and cleanly.

5. **Release creation:** Used `gh release create v0.2.0 --notes-file release-notes.md --latest`. Confirmed live via `gh release list`. GitHub Actions release.yml will auto-trigger on `v*` tag to build cross-platform binaries.

6. **No CI wait:** Did not wait for CI binary builds before creating the release — per task spec, the workflow picks up the tag automatically.

**Outcome:** v0.2.0 live at https://github.com/macro88/gigabrain/releases/tag/v0.2.0. Release is marked Latest. Tag v0.2.0 pushed. Version bump committed to main.

## 2026-04-17 Phase 3 Task 8.3 — Skills Review

**Role:** Reviewer (task 8.3)

**What happened:**
- Reviewed all five Phase 3 SKILL.md files for completeness, clarity, and agent-executability.
- All five approved: briefing, alerts, research, upgrade, enrich.
- Resolved the 30-day vs. 90-day stale threshold discrepancy Amy flagged.

**Stale threshold ruling:**
- Spec scenario (`specs/skills/spec.md` line 28) says **30 days** — this is the BDD scenario and governs.
- Task 1.2 description text said "90 days" — this was an authoring error in the task summary, not the spec.
- `alerts/SKILL.md` uses 30 days → **correct**. No change to skill file required.
- Corrected task 1.2 description text in `tasks.md` from ">90 days" to ">30 days (timeline_updated_at > truth_updated_at by 30+ days)".

**Task 8.3 marked `[x]` in tasks.md.**

**Decision note written to:** `.squad/decisions/inbox/leela-phase3-skills-review.md`

**Learnings:**
