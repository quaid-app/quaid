## 1. Initialization Defaults

- [ ] 1.1 Update DB initialization logic so fresh `quaid init` sets default collection root to `~/.quaid/vault` and keeps it as write-target.
- [ ] 1.2 Ensure initialization creates the resolved default root directory when missing and surfaces clear errors on permission failures.
- [ ] 1.3 Add guard logic so existing databases with already-valid writable write-target roots are not modified.

## 2. Conversation Write Path Compatibility

- [ ] 2.1 Align conversation turn write preconditions with the new first-run default behavior so fresh initialized DBs no longer fail with missing writable root.
- [ ] 2.2 Add/adjust fallback handling for legacy unconfigured default states according to the approved migration rule.
- [ ] 2.3 Verify no changes to multi-collection ambiguity routing or write-target exclusivity semantics.

## 3. Tests

- [ ] 3.1 Add integration tests for fresh init defaults validating `root_path`, `is_write_target`, and directory creation.
- [ ] 3.2 Add conversation tool tests proving `memory_add_turn` succeeds on a fresh initialized DB without manual collection bootstrap.
- [ ] 3.3 Add regression tests ensuring existing configured DBs are preserved and not rewritten.

## 4. Documentation

- [ ] 4.1 Update getting-started and operator docs to describe default root behavior at `~/.quaid/vault`.
- [ ] 4.2 Document compatibility behavior for existing DBs and manual override paths for custom collection roots.
- [ ] 4.3 Add release-note/changelog entry for first-run sensible collection defaults.

## 5. Validation and Rollout Readiness

- [ ] 5.1 Run full relevant test suites and confirm no regressions in collection and conversation flows.
- [ ] 5.2 Validate CLI, MCP, and playground first-run smoke tests against a clean environment.
- [ ] 5.3 Prepare rollback notes for disabling conditional bootstrap if unexpected regressions appear.
