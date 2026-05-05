# Bender: SLM Model Lifecycle — Proof Revision

**From:** Bender (Tester)
**Date:** 2025-01-30
**Commit:** `be32993`
**Branch:** `feat/slm-conversation-mem`
**Closes defects from:** Professor's rejection of `875cdd8`

---

## What was fixed

Professor rejected `875cdd8` with two defects.  Both are now closed.

### Defect 1 — Curated-alias "source-pinned" guarantee unproved

**Root cause:** All pre-existing integration tests used a raw `"org/model"` repo-id as
the alias.  That path calls `install_model_into_dir` (unpinned/manifest-only) and sets
`verified_from_source = false`, so `source_pinned` is always `false`.  The tests at
lines 375 and 445 actually *asserted* `!source_pinned` — they proved the *un*pinned
path, not the curated path.

**Fix applied:**
1. Added a `#[cfg(any(test, feature = "test-harness"))]` curated alias stub named
   `"test-pinned"` directly in `model_lifecycle.rs`.  The stub has mixed
   SHA-256/git-blob-SHA1 digest pins computed against the standard `mock_files(false)`
   fixture content so no real network traffic is required.
2. Added a `test-harness` Cargo feature (`Cargo.toml`).  `#[cfg(test)]` alone cannot
   activate code inside library crates when they are compiled for integration tests in
   `tests/` — the library is compiled as a non-test crate in that scenario.  The feature
   flag is the correct mechanism.  Integration tests run with
   `--features bundled,online-model,test-harness`.
3. Added three integration tests:
   - `download_curated_alias_sets_source_pinned` — happy path; asserts
     `status.source_pinned = true` and that exactly 3 file GETs are made (the curated
     path must skip the metadata API).
   - `download_curated_alias_rejects_tampered_sha256_file` — weight file bytes replaced
     by attacker content; must return an error containing `"integrity check failed"` and
     must clean up the partial cache directory.
   - `download_curated_alias_rejects_tampered_git_blob_file` — config/tokenizer bytes
     replaced; same rejection guarantee.
4. Added `mock_files_with_bad_file(bad_file, bad_content)` helper for the rejection tests.
5. Added 4 unit tests for `verify_source_pin` directly (both digest variants, both
   accept/reject branches) so the digest logic is proved in isolation.

### Defect 2 — Task 3.2 wording mismatches shipped contract

**Root cause:** The task read "runs SHA-256 integrity checks" which implies a
single-digest scheme, but the shipped code uses a mixed scheme: SHA-256 for weight
files (`.safetensors`, `.model`) and git-blob-SHA1 for metadata files (`.json`).

**Fix applied:** Updated `openspec/changes/slm-extraction-and-correction/tasks.md`
task 3.2 to read:

> *runs per-file source-pinned digest verification (SHA-256 for weight files,
> git-blob-SHA1 for metadata/tokenizer files) for curated aliases; server-supplied
> ETag SHA-256 checks for raw repo downloads*

---

## Key design decisions

**Why `#[cfg(any(test, feature = "test-harness"))]` instead of a runtime env-var?**
The `PinnedDigest` enum uses `&'static str` for digest values (required because the
production curated-file tables are `&'static [SourcePinnedFile]`).  Runtime strings
cannot be used.  Compile-time constants gated behind a feature flag are the only way
to inject test fixtures into the same enum without changing the production type.

**Why a separate `test-harness` feature instead of reusing `online-model`?**
`online-model` controls network-download code paths and is expected to be available in
production online builds.  Bundling test fixtures into a production binary under that
feature would be wrong.  `test-harness` is explicitly not in `default` and its
description marks it as for integration testing only.

**Why the curated path skips the metadata API?**
`source_pins_for_alias` returns the file list directly from the pinned manifest — the
server's file listing is untrusted for curated aliases.  The 3-request assertion in the
happy-path test verifies this invariant holds.

---

## Test results

```
cargo test --test model_lifecycle --no-default-features \
  --features bundled,online-model,test-harness

running 12 tests
test download_curated_alias_sets_source_pinned .............. ok
test download_curated_alias_rejects_tampered_sha256_file .... ok
test download_curated_alias_rejects_tampered_git_blob_file .. ok
... (9 pre-existing tests) ...
test result: ok. 12 passed; 0 failed
```

Unit tests: 980 passed, 0 failed (all pre-existing tests unaffected).
