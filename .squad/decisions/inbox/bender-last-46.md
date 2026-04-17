### PR #46 Final Validation — Bender

**Date:** 2026-04-17
**By:** Bender (Tester)
**Verdict:** ✅ APPROVE

**Scope:** Final test/validation review of Scruffy's revision (1da8443) — the last claimed seam removal.

---

**The fake seam is gone.**

Old T19 overrode `detect_profile()` with a canned stub that returned 1 and printed a hardcoded warning. The failure was synthetic — it never exercised the production `detect_profile` code path through `main()`.

New T19 (Scruffy, 1da8443):
- Re-sources `install.sh` at line 371, restoring ALL production functions including `detect_profile`.
- Creates a real unwritable directory (`chmod 500`) and sets `HOME` to it.
- Calls `main()` — the real entry path.
- Production `detect_profile` runs, hits the real filesystem constraint, and fails genuinely.
- `write_profile` propagates the failure to `main()`, which exits non-zero with recovery output.

**Only network I/O is stubbed** (curl, resolve_version, resolve_platform, resolve_channel, verify_checksum, need_cmd). These are the correct stubs for deterministic testing. No profile/write functions are faked.

**Assertions verified (T19/T19b/T19c):**
1. `main()` exits non-zero ✓
2. stderr contains real `detect_profile` warning: `Cannot create shell profile <path>/.zshrc` ✓
3. stderr contains `main()` failure message: `gbrain was installed, but PATH/GBRAIN_DB were not persisted automatically.` ✓
4. stderr contains manual recovery hints (PATH, GBRAIN_DB exports) ✓
5. stderr contains sandboxed/two-step install hint ✓
6. stdout still reports `Installed gbrain to` (binary was installed before profile failed) ✓
7. Profile file was NOT created in the unwritable directory ✓

**Validation evidence:**
- Local run: 25/25 tests pass, 0 failures.
- CI (commit 1da8443): All 12 check runs green — Check, Test (including `sh tests/install_profile.sh`), Coverage, Benchmarks.
- Codecov: 86.98%, no coverage regression.

**Non-blocking note:**
- `openspec/changes/simplified-install/proposal.md` still says "does not modify any shell files." Stale after this PR. Not a code defect — OpenSpec doc nit for Hermes.

**Outcome:** The installer failure-path contract is proven end-to-end through the real `main()` function with real filesystem constraints. No remaining test seams bypass production code. Cleared for merge.
