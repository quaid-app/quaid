# Gate-vs-Bail Test Discipline

**Skill type:** Edge-case testing pattern  
**Applies to:** Any feature that is temporarily disabled at the dispatch layer (CLI bail, early return, feature flag)

---

## The pattern

When a feature is backed out by inserting an early bail (e.g., `bail!("feature X is deferred")`), tests that were written to prove the feature's *correctness behaviors* now only prove that the bail fires. This is **confusing and dangerous**:

1. The test name implies the behavior is tested; the assertion proves only the guard.
2. If the bail message changes, the substring assertion passes vacuously.
3. When the feature is re-enabled, the tests look green but verify nothing.

This pattern **used to appear** in `tests/quarantine_revision_fixes.rs` before quarantine restore was re-enabled:

```rust
// These tests are named for behaviors that ARE NOT tested:
restore_surface_is_deferred_for_non_markdown_target    // bail fires before .md check
restore_surface_is_deferred_for_live_owned_collection  // bail fires before ownership gate
restore_surface_is_deferred_before_target_conflict_mutation  // bail fires before conflict check
restore_surface_is_deferred_for_read_only_collection   // bail fires before writable check
```

That specific restore example has now graduated to real behavior proofs (`restore_refuses_when_target_appears_after_the_earlier_absence_check`, `restore_rollback_unlinks_residue_and_fsyncs_parent_before_returning`, etc.). Keep using the pattern for future backed-out features, but do not cargo-cult bail assertions once the production seam exists.

---

## Rules

### Rule 1: Test names must match what is actually asserted

If a test only proves the bail fires, name it for the bail:

```
// BAD
fn restore_surface_is_deferred_for_non_markdown_target()
// GOOD  
fn restore_command_returns_deferred_error_regardless_of_target_extension()
```

### Rule 2: Mark bail-testing tests explicitly

Add a comment at the top of any test that asserts a deferred-surface bail:

```rust
// NOTE: This test asserts the deferred-surface bail only.
// The actual behavior (non-.md extension validation) is NOT tested here.
// When restore is re-enabled, this test must be updated to assert real behavior.
#[test]
fn restore_command_returns_deferred_error_regardless_of_target_extension() { ... }
```

### Rule 3: Write the real test too (even if it skips)

When backing out a feature, write the real behavior test with `#[ignore]` or a feature-gate skip:

```rust
#[test]
#[ignore = "quarantine restore is deferred; re-enable when restore lands (see openspec tasks 9.8, 17.5j)"]
fn restore_rejects_non_markdown_target_extension() {
    // This test proves the real behavior once restore is active.
    ...
    assert!(
        output_text.contains("target path must have a .md extension"),
        ...
    );
}
```

This way:
- The behavior spec exists in code
- It cannot accidentally pass under the bail
- When restore lands, `cargo test --ignored` immediately surfaces what needs re-verification

### Rule 4: Distinguish `assert!(text.contains(...))` from structural error matching

A bail test that checks for a substring of an error message is inherently fragile — any wording change silently invalidates the proof. Prefer structural assertions when possible:

```rust
// Fragile: fails silently if message changes  
assert!(stderr.contains("quarantine restore is deferred"));

// More robust: tie to the specific error code or command exit behavior
assert!(!result.status.success(), "restore must fail");
assert!(stderr.contains("QuarantineRestoreDeferredError"), "must surface restore-deferred error code");
```

---

## Application to this codebase

### `tests/quarantine_revision_fixes.rs`

This file is the canonical example of how the pattern should end:

1. Once Fry exposed deterministic hooks, the bail-only restore assertions were deleted.
2. The parked behavior names became real, targeted proofs against concrete error codes and state invariants.
3. The remaining restore deferrals are now genuinely broader-scope items (audit/live routing/overwrite policy), not placeholder assertions pretending those behaviors are covered.

### Known gap: positive discard path

`blocker_1_failed_export_does_not_unlock_discard` proves the negative (failed export → still blocked). The positive (successful export → discard allowed with db_only_state) is also untested. **Both sides of a boolean condition should be tested.** This is a special case of the bail-test pattern: only the error path is covered.

---

## Quick checklist before landing tests for deferred features

- [ ] Does the test name accurately describe what is actually asserted?
- [ ] Does the test assert the real behavior, OR only the early-exit/bail behavior?
- [ ] If bail-only: is there a companion `#[ignore]` test for the real behavior?
- [ ] If bail-only: is there a `// NOTE: asserts bail only, not the named behavior` comment?
- [ ] Is the bail message assertion a stable error code, or a substring of a human-readable message?
- [ ] Are both the positive and negative cases of the feature's guard tested?
