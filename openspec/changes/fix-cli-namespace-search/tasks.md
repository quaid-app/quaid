# Tasks

## 1. Reproduce and confirm the bug

- [ ] 1.1 Add a failing CLI integration test for issue #145 with matching pages in global,
  `workns`, and `otherns`.
- [ ] 1.2 Assert `quaid --json search --namespace workns bitcoin` returns only global plus
  `workns` pages.
- [ ] 1.3 Assert `quaid --json search bitcoin` returns only global pages when `--namespace`
  is omitted.
- [ ] 1.4 Run the new test before the fix and confirm it fails if the issue is present on
  the implementation branch.

## 2. Fix CLI search namespace propagation

- [ ] 2.1 Inspect `src/main.rs` search dispatch and ensure it passes
  `namespace.as_deref().or(Some(""))` to `commands::search::run`.
- [ ] 2.2 Inspect `src/commands/search.rs::run()` and ensure it validates the original
  namespace input before normalization.
- [ ] 2.3 Ensure `src/commands/search.rs::run()` normalizes omitted namespace with
  `let namespace = namespace.or(Some(""));`.
- [ ] 2.4 Ensure the sanitized path calls `search_fts_canonical_tiered_with_namespace`
  with that normalized namespace.
- [ ] 2.5 Ensure the `--raw` path calls `search_fts_canonical_with_namespace` with that
  normalized namespace.
- [ ] 2.6 Remove or replace any non-namespace FTS helper call in the CLI search path.

## 3. Verify core FTS predicate

- [ ] 3.1 Inspect `src/core/fts.rs::search_fts_internal()`.
- [ ] 3.2 Confirm `Some("")` appends `AND p.namespace = ?` with an empty-string parameter.
- [ ] 3.3 Confirm `Some("workns")` appends `AND (p.namespace = ? OR p.namespace = '')`.
- [ ] 3.4 Confirm `None` appends no namespace predicate and remains an internal
  all-namespace mode only.
- [ ] 3.5 Add or update a focused unit test for
  `search_fts_canonical_with_namespace` if the CLI integration test does not make the
  failing branch obvious.

## 4. Release-channel check

- [ ] 4.1 Confirm no `embedded-model` or `online-model` cfg affects `src/main.rs`,
  `src/commands/search.rs`, or `src/core/fts.rs`.
- [ ] 4.2 Run the regression test under the default airgapped feature set.
- [ ] 4.3 If CI supports the online feature set, run the same test with
  `--no-default-features --features bundled,online-model`.
- [ ] 4.4 Document that airgapped and online binaries share the same FTS namespace filter
  code path for `quaid search`.

## 5. Verification

- [ ] 5.1 Run `cargo fmt --all --check`.
- [ ] 5.2 Run the new CLI namespace search integration test.
- [ ] 5.3 Run `cargo test` or the closest available namespace/search test subset.
- [ ] 5.4 Manually verify:
  `quaid --db <temp.db> --json search --namespace workns bitcoin`.
- [ ] 5.5 Close GitHub issue #145 after the fix lands.
