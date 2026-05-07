## Context

The current crate documentation surface is effectively empty:

- `src/lib.rs` is three `pub mod` declarations with no `//!` crate doc.
- Roughly 40 source files under `src/core/` and `src/mcp/` open with code; only a handful (`fts.rs`, the recently-touched `conversation/queue.rs`) carry meaningful doc comments. `vault_sync.rs` has ~73 `pub` items and almost no `///` docs (per `docs/CODE_REVIEW.md` §6.1).
- A rough census shows ≈500 `pub` items across `core/` and `mcp/` once #4 and #5 land — every one of them needs at least a one-sentence `///`.
- There is no CI gate on doc warnings, so even good doc comments rot silently.

This is documentation-only work (no behavior change, no schema change, no wire-format change), but the surface area is large and the diff is mechanically dense — the design questions are about scope, ordering, lint enforcement, and how to keep a moving target documentable while #4 and #5 are still in flight.

Stakeholders: maintainers reading rustdoc, future contributors orienting on the codebase, and CI (which gains a doc-warning gate). End-users of the binary are unaffected.

## Goals / Non-Goals

**Goals:**

- Crate landing page (`cargo doc --open`) opens to a meaningful `lib.rs` `//!` intro that orients a first-time reader on what Quaid is, the three top-level modules, and where to start reading.
- Every module file under `src/core/` and `src/mcp/` opens with a `//!` paragraph naming its single responsibility plus a one-line "see also" cross-reference.
- Every public item (`pub fn`, `pub struct`, `pub enum`, `pub trait`, plus their public methods) reachable from `src/core/` and `src/mcp/` carries a `///` doc with at least one sentence describing intent (not restating the signature).
- `#![warn(missing_docs)]` enabled on `src/lib.rs`, and `cargo doc --no-deps` builds with zero warnings under `RUSTDOCFLAGS="-D warnings"`.
- A CI step enforces the zero-warning invariant on every PR.

**Non-Goals:**

- Documenting private items (no `#![warn(missing_docs)]` escalation beyond the default which already covers `pub`-reachable items).
- Documenting `src/commands/` or `src/main.rs` — these are CLI entry points whose user-facing contract is clap help text, not rustdoc.
- Documenting examples, tests, or build scripts.
- Adding worked-example `# Examples` blocks. One sentence per item is the bar; richer examples can be a follow-on if value warrants.
- Producing or hosting external rustdoc HTML (e.g., publishing to docs.rs). This change makes rustdoc *clean*; publishing is a separate decision.
- Refactoring code to make it more documentable. If something is hard to explain in a sentence, the doc records that fact; restructuring belongs in its own change.

## Decisions

### D1. Sequence after #4 and #5, not before

**Decision:** Block on `decompose-vault-sync-module` (#4) and `decompose-mcp-server-module` (#5) being archived before starting implementation work on this change.

**Why:** The two largest surfaces — `vault_sync.rs` (~73 `pub` items, 12 KLOC) and `mcp/server.rs` (~26 `pub` items, large) — are about to be split into ~9 cohesive sub-modules each. Documenting them before the split would mean rewriting every module-level `//!` block (the responsibility line is the whole point of the doc) and re-locating every `///` as items move files. The split also produces *better* documentation targets: a 600-line module with one job has a one-sentence `//!`; a 12 KLOC monolith does not.

**Alternatives considered:**

- *Document now, accept the rework.* Rejected: roughly 100 pub items move between files in #4 alone; the diff churn would dominate the value.
- *Document everything except `vault_sync.rs` and `mcp/server.rs` now, fill them in after the splits.* Rejected: violates the "zero warnings" goal — `#![warn(missing_docs)]` would either need module-level allow attrs (which then need to be removed and re-reviewed) or the lint stays off until completion. Either way, no real progress on the CI gate until the splits land. Cleaner to wait.

### D2. `#![warn(missing_docs)]`, not `#![deny(missing_docs)]`

**Decision:** Use `#![warn(missing_docs)]` on `src/lib.rs`, paired with `RUSTDOCFLAGS="-D warnings"` in the CI doc job.

**Why:** Local `cargo build` stays warning-not-error so contributors aren't blocked by missing docs on a half-finished `pub fn` mid-edit; CI promotes warnings to errors so nothing un-doc'd merges. This is the conventional Rust pattern and matches what `add-rust-lints-and-ci-gate` does for clippy.

**Alternatives considered:**

- *`#![deny(missing_docs)]` directly.* Rejected: hostile to in-progress local development; nothing is gained over the warn-plus-CI-deny pattern.
- *Per-module gates.* Rejected: bookkeeping cost. Crate-wide is the simpler invariant.

### D3. Module `//!` doc shape: one paragraph + one-line "see also"

**Decision:** Every `src/core/*.rs` and `src/mcp/*.rs` (and their submodules) opens with:

```rust
//! <One-paragraph statement of the module's single responsibility,
//! written so a maintainer scanning rustdoc can decide in 10 seconds
//! whether this is the file they need.>
//!
//! See also: [`super::other_module`] for <adjacent concern>,
//! [`crate::core::something`] for <related concern>.
```

**Why:** Constrains the writer (no creeping design narrative; that goes in `design.md`), gives the reader a consistent shape, and the "see also" line buys most of the value of a hand-curated module map without us maintaining one centrally. Path links are checked by rustdoc, so broken cross-references surface as warnings — matches the D2 CI gate.

**Alternatives considered:**

- *Free-form module docs.* Rejected: drift across 40 files; review becomes case-by-case.
- *Centralized module map in `lib.rs` instead of per-file `//!`.* Rejected: maintainers usually arrive at a file via grep or click-through, not by reading `lib.rs` top-down. Per-file docs are where they look.

### D4. Item `///` doc bar: ≥ one sentence, intent over signature

**Decision:** Every `pub fn` / `pub struct` / `pub enum` / `pub trait` (and each `pub` method on a `pub` impl) carries at least one `///` line describing *what role the item plays*, not what its parameters are. Style match: `core/fts.rs` (`/// Expands a sanitized multi-token query into an explicit FTS5 OR chain.`) and `core/conversation/queue.rs` (`enqueue` / `enqueue_force_path`).

**Why:** The signature already encodes types and arity; the doc adds the missing layer (intent, invariants worth knowing, expected caller). One sentence is the floor, not a ceiling — when an item has a non-obvious invariant or precondition, the doc grows. But a uniform floor lets us mechanically clear the lint and review the harder cases on their own merits.

**Anti-patterns to avoid:**

- `/// Returns the foo.` on `pub fn foo() -> Foo` — no signal added. Replace with intent: what is the foo, when is it valid, what does the caller do with it.
- `/// Get/set the bar.` on accessors — only useful if there's a non-obvious invariant; otherwise reach for a clearer struct field name.
- Restating the parameter list. `# Arguments` sections are reserved for items with non-obvious parameter contracts (e.g., units, ranges, mutual exclusivity).

**Alternatives considered:**

- *Require `# Examples` on every public item.* Rejected: not enforceable as a lint, doubles to triples the per-item word count, and most internal-library items don't need them. A targeted follow-up can add examples to the items consumers actually call (`memory_*` MCP tools, primary `core::search::hybrid_search`).
- *Free-text length.* Rejected: review becomes "is this enough" instead of "is this true."

### D5. CI enforcement via `cargo doc --no-deps` with `-D warnings`

**Decision:** Add a CI step:

```yaml
- name: rustdoc
  env:
    RUSTDOCFLAGS: "-D warnings"
  run: cargo doc --no-deps --all-features
```

If `add-rust-lints-and-ci-gate` is already in CI when this change implements, slot the step into that job; otherwise add a sibling job. Either way, doc warnings become PR-blocking.

**Why:** `--no-deps` keeps runtime tractable (no doc-building the dep tree), `-D warnings` is the only mechanism to turn the `warn(missing_docs)` lint into a real gate, `--all-features` exercises both the `embedded-model` and `online-model` channels (per `CLAUDE.md` "embedding model" section). Local contributors run `cargo doc` without the env var and see warnings, not errors.

**Risks:** `cargo doc` is slower than `cargo check`. With `--no-deps` and a warm Cargo cache it's typically under 30s on this crate. If it becomes a bottleneck, gate it on the same triggers as the lint job.

### D6. Out-of-scope content stays out

**Decision:** No documentation of private items, `commands/`, `main.rs`, `tests/`, `examples/`, or build scripts. No new architecture diagrams. No tutorial content. No CHANGELOG-style notes in doc comments.

**Why:** Scope discipline. Private items have `cargo doc --document-private-items` for the rare cases someone needs them. CLI commands have clap help. Tutorial-grade content lives in `skills/*/SKILL.md` (per `CLAUDE.md` "thin harness, fat skills"). Mixing those into rustdoc dilutes the rustdoc reader's value and forces parallel maintenance.

## Risks / Trade-offs

- **Risk: docs drift from code.** A doc comment is true on the day it's written and may not be later. → Mitigation: the CI gate doesn't catch *staleness*, only *absence*. Accept this; the alternative (heavyweight doc tests on every item) is disproportionate. The "see also" links *are* checked by rustdoc, so structural drift fails CI.
- **Risk: low-quality "rubber stamp" docs that satisfy the lint without adding signal.** → Mitigation: review process. The implementation tasks split work by module so reviewers can apply judgment per area. The good examples in `core/fts.rs` and `core/conversation/queue.rs` give a calibration target. Reviewers should reject `/// Returns the foo.`-style docs.
- **Risk: blocks on #4 and #5.** If those changes slip, this one slips. → Mitigation: tracked as an explicit dependency in the proposal (`Impact › Sequencing`); apply-time check before starting work. If #4/#5 are stalled, this change can be deferred without partial-state cost.
- **Risk: large mechanical PR is hard to review carefully.** → Mitigation: tasks.md groups work by module — `core/db`, `core/search/*`, `core/conversation/*`, `core/vault_sync/*` (post-#4), `mcp/*` (post-#5), `lib.rs` + CI. A reviewer can take one section at a time. If the diff is still too large for one PR, split along those task boundaries.
- **Trade-off: lint covers existence, not quality.** Accepted. Quality is a review concern, not a CI concern. The bar is one sentence of *intent*; reviewers enforce it.
- **Trade-off: `cargo doc` adds CI time.** Acceptable: ~30s with `--no-deps`. Smaller than the existing test job.

## Migration Plan

1. **Pre-flight (apply-time):** Verify `decompose-vault-sync-module` and `decompose-mcp-server-module` are archived. If not, stop and surface this to the caller — do not begin work.
2. **Crate root:** Add `//!` intro and `#![warn(missing_docs)]` to `src/lib.rs`. Run `cargo doc --no-deps` locally, expect warnings (every undocumented `pub` item is one). This is the baseline workload.
3. **Module-by-module:** For each file under `src/core/` and `src/mcp/` (in the task-group order), add the `//!` header then walk every `pub` item and add a `///` doc. Re-run `cargo doc --no-deps 2>&1 | grep warning` after each module to track progress and catch regressions early. Commit per module group.
4. **CI gate:** Add the `cargo doc` step (per D5) and verify it passes on the branch.
5. **Final sweep:** `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features` must succeed locally before requesting review.
6. **Rollback:** Pure addition; revert is `git revert` of the commit range. No data, schema, or config touched. The CI step can be reverted independently of the doc commits if it proves flaky.

## Open Questions

- **Does the `online-model` cfg gate any `pub` items that need docs only in that build?** Spot check during implementation: `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --no-default-features --features bundled,online-model` must also be clean. If it surfaces feature-gated `pub` items, they get docs too. Decision recorded here so the implementer remembers to run both feature combinations.
- **Should the crate `//!` link to `skills/`?** Probably not — `skills/` is for agents, rustdoc is for humans (per the proposal's "What Changes"). But a sentence acknowledging the existence of `skills/` and pointing readers there for *workflow* questions (vs. *API* questions) may save support load. Implementer decides; if included, keep it to one sentence.
- **Item-level granularity for re-exports.** `pub use` re-exports inherit the source item's docs by default. If a re-export's intent differs from the source (e.g., narrowing to a public façade), it gets its own `///`. Otherwise leave the inherited doc.
