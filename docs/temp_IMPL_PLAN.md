# Implementation Plan — Code Review Follow-up

Working document. Tracks the seven OpenSpec proposals that close out
`docs/CODE_REVIEW.md` once the three correctness fixes in
`fix-extraction-force-correctness` land. Delete this file once every proposal
is archived.

## Status legend

- 🟢 ready — no upstream dependencies
- 🟡 staged — depends on an earlier proposal
- ⚫ deferred — not its own proposal; folded into another or low-priority

All seven proposals have been created under [openspec/changes/](../openspec/changes/) and are apply-ready.

## Sequencing

```
#1 lints/CI ──┬─→ #2 remove-panics
              └─→ #3 extract-tests ──┬─→ #4 decompose-vault-sync
                                     └─→ #5 decompose-mcp-server
                          (independent) #6 collapse-search-fns
                          (after #4,#5) #7 public-api-rustdoc
```

Estimated wall-clock from start of #1 to end of #7, assuming one engineer
working in series and no surprises: **2–3 weeks**. Mechanical proposals (#1,
#2, #3, #6) are each <1 day. Structural proposals (#4, #5) are each 2–4 days.
Docs (#7) is half a day once the modules are stable.

## Quick reference — proposals

| # | Name | Status | Covers (§) | Effort |
|--:|---|:--:|---|--:|
| 1 | [`add-rust-lints-and-ci-gate`](../openspec/changes/add-rust-lints-and-ci-gate/) | 🟢 | 3.1, 3.2, 3.3, 7 partial | <1d |
| 2 | [`remove-production-panic-paths`](../openspec/changes/remove-production-panic-paths/) | 🟡 (after #1) | 2.1 | <1d |
| 3 | [`extract-inline-tests-to-integration`](../openspec/changes/extract-inline-tests-to-integration/) | 🟢 | 1.5, 4.2 partial | 1–2d |
| 4 | [`decompose-vault-sync-module`](../openspec/changes/decompose-vault-sync-module/) | 🟡 (after #3) | 1.3, 2.2, 2.3, 5.3 partial | 3–4d |
| 5 | [`decompose-mcp-server-module`](../openspec/changes/decompose-mcp-server-module/) | 🟡 (after #3) | 1.4, 2.4 | 2–3d |
| 6 | [`collapse-search-fn-variants`](../openspec/changes/collapse-search-fn-variants/) | 🟢 | 1.6, 5.1, 5.2 | <1d |
| 7 | [`add-public-api-rustdoc`](../openspec/changes/add-public-api-rustdoc/) | 🟡 (after #4, #5) | 6.1 | <1d |

Deferred (no proposal):

| Item | Disposition |
|---|---|
| §5.3 longest functions | Folded into #4 (vault_sync) and a future `split-put-command` |
| §5.4 main.rs dispatch | Trivial — fold into #1 or land as a one-shot PR |
| §7 perf optimization | Defer until profiling justifies; redundant-clone sweep handled by #1 |

---

## Proposal #1 — `add-rust-lints-and-ci-gate`

**Goal:** Surface the existing class-of-bug findings (`unwrap`, `panic`,
`large_enum_variant`, `redundant_clone`, dead `#[allow]`s) automatically and
prevent regressions via CI. This is the highest-leverage cheap win because it
turns several follow-up proposals into "fix what clippy already flagged"
rather than "audit by hand."

**Acceptance criteria:**

- `Cargo.toml` has `[lints.rust]` and `[lints.clippy]` tables matching
  Apollo's defaults from `docs/CODE_REVIEW.md` §3.1.
- `clippy.toml` and/or `rustfmt.toml` exist if any per-rule tuning is
  required (e.g. `unwrap_used` overrides for `#[cfg(test)]`).
- A CI step in `.github/workflows/` runs
  `cargo clippy --all-targets --all-features --locked -- -D warnings` and
  `cargo fmt --all -- --check` on every PR.
- Existing `#[allow(...)]` annotations are migrated to `#[expect(...)]` with a
  one-line `// reason:` comment per Rust 1.81 best practice (`docs/CODE_REVIEW.md` §3.2).
- All warnings flagged on the current `main` branch are either fixed in this
  PR or explicitly deferred to proposal #2 with a tracking comment.

**Prompt to feed `/opsx:propose`:**

```
/opsx:propose Establish a Rust lints + CI gate for the Quaid repo. The
existing code review at docs/CODE_REVIEW.md §3 (and the Apollo Rust Best
Practices Handbook it cites) recommends:

- Add [lints.rust] to Cargo.toml: unsafe_code = "forbid", missing_docs =
  "warn", unreachable_pub = "warn".
- Add [lints.clippy] to Cargo.toml: clippy::all = warn (priority -1),
  clippy::pedantic = warn (priority -1), plus explicit warns for
  unwrap_used, expect_used, panic, print_stdout, redundant_clone,
  needless_collect, large_enum_variant.
- Migrate the 46 existing #[allow(...)] annotations to #[expect(...)] (Rust
  1.81+) so dead suppressions surface as warnings; each gets a one-line
  // reason: comment.
- Add a CI workflow step in .github/workflows/ running
  `cargo clippy --all-targets --all-features --locked -- -D warnings` and
  `cargo fmt --all -- --check` on every PR.

Scope this proposal to the lint/CI infrastructure plus any warnings that
fall out trivially. Do NOT fold in the production unwrap fixes at
src/core/vault_sync.rs:3380 or src/mcp/server.rs:1793/1880/1892/1990 — those
go in a follow-up proposal (#2 in docs/temp_IMPL_PLAN.md). For warnings
that need real engineering work, add a one-line code comment with a
TODO referencing the follow-up proposal name and #[allow(...)] the lint
locally. The goal is "lints are green on main" before #2 starts.

Reference: docs/CODE_REVIEW.md §3.1, §3.2, §3.3.
```

---

## Proposal #2 — `remove-production-panic-paths`

**Goal:** Eliminate the panic paths that the lints from #1 surface, focusing
on the two real-risk cases identified in §2.1 of the original review and any
others #1 flags as legitimate (not test-gated).

**Acceptance criteria:**

- `src/core/vault_sync.rs:3380` no longer panics when `pending_root_path`
  is `None` while `manifest_json.is_some()` — replaced with either a
  let-else returning `FinalizeOutcome::NoPendingWork` or a new
  `VaultSyncError::InvariantViolation { ... }` variant.
- The four `serde_json::to_string_pretty(...).unwrap()` cases in
  `src/mcp/server.rs` (lines 1793, 1880, 1892, 1990 per the original
  review) propagate via `map_anyhow_error` (or a new `serialize_response`
  helper).
- The 6 mutex `lock().unwrap()` cases in `src/core/inference.rs` use the
  existing `unwrap_or_else(|e| e.into_inner())` pattern from
  `src/mcp/server.rs:1804`.
- After this change, a clean `cargo clippy --all-targets -- -D warnings`
  has zero `unwrap_used` / `expect_used` / `panic` warnings outside
  `#[cfg(test)]`.
- Each fix has a regression test where the failure path is reachable
  (notably `vault_sync.rs:3380`'s invariant case).

**Prompt to feed `/opsx:propose`:**

```
/opsx:propose Eliminate production panic paths in Quaid surfaced by the
lints landed in proposal #1 (add-rust-lints-and-ci-gate). The targets are
documented in docs/CODE_REVIEW.md §2.1:

1. src/core/vault_sync.rs:3380 — `let pending_root_path =
   PathBuf::from(collection.pending_root_path.clone().unwrap());`. The
   two lines above check `manifest_json.is_some()` but
   `pending_root_path` can independently be None mid-restore. Replace
   with a let-else returning `FinalizeOutcome::NoPendingWork`, OR a new
   `VaultSyncError::InvariantViolation` variant if the call graph
   guarantees it should never fire (verify by reading
   begin_restore/finalize_pending_restore — the original review noted
   the surrounding code does not enforce the invariant).

2. src/mcp/server.rs:1793, 1880, 1892, 1990 — four
   `serde_json::to_string_pretty(...).unwrap()` calls. The inputs are
   structurally JSON-safe so the unwrap is technically infallible, but
   the panic path is wrong on principle and one line shorter to remove
   once a `serialize_response` helper exists. Add the helper, route the
   four sites through it, propagate via `?` and the existing
   map_anyhow_error.

3. src/core/inference.rs — 6 `lock().unwrap()` cases on Mutex guards.
   Convert to the existing pattern at src/mcp/server.rs:1804:
   `lock().unwrap_or_else(|e| e.into_inner())`. PoisonError-recovery is
   a defensible local choice for these hot-path mutexes.

Each fix needs a regression test where the failure path is actually
reachable. The vault_sync one is the most important — if you can't
construct a test that hits the invariant, document why in the test
module comment and add a `debug_assert!` to make the implicit invariant
explicit.

Do not fold in any structural refactor (file splits, error type
splits). The scope is "remove panics surfaced by lints."

Reference: docs/CODE_REVIEW.md §2.1.
```

---

## Proposal #3 — `extract-inline-tests-to-integration`

**Goal:** Halve the line count of the eight monoliths in `src/` by moving
public-API `mod tests` blocks to `tests/` integration files. Pure mechanical;
the unblocking step for #4 and #5.

**Acceptance criteria:**

- After the change, no production source file in `src/` has a
  `#[cfg(test)] mod tests { ... }` block longer than ~500 lines.
- White-box tests (those that need access to private items) stay inline.
- Each migrated test is split by feature, not by source file: e.g.
  `vault_sync.rs:5909`'s 6,596-line `mod tests` becomes
  `tests/vault_sync_ipc.rs`, `tests/vault_sync_restore.rs`,
  `tests/vault_sync_watcher.rs`, etc., one file per concern.
- `cargo test` passes before and after the migration; no test is
  silently lost.
- `tests/collection_cli_truth.rs` (108 KB) is split by command into
  `tests/cli_collection_add.rs`, `tests/cli_collection_sync.rs`, etc.
- The migration is a series of self-contained commits, one per source
  file, so a bisect remains useful.

**Prompt to feed `/opsx:propose`:**

```
/opsx:propose Move the inline `#[cfg(test)] mod tests { ... }` blocks
out of the eight monolith production files in src/ into integration
test files under tests/. This is a pure mechanical refactor with zero
behavior change; its purpose is to unblock the larger module-split
proposals (#4 split-vault-sync, #5 split-mcp-server in
docs/temp_IMPL_PLAN.md) by halving the line count of the worst offenders
without touching production logic.

Sources to migrate, with their current line counts (per
docs/CODE_REVIEW.md §1.1, §1.5):

| File | mod tests starts at | LOC |
|---|---|---|
| src/core/vault_sync.rs | line 5909 | 6,596 |
| src/core/reconciler.rs | line 3119 | 4,285 |
| src/mcp/server.rs | line 2016 | 3,888 |
| src/commands/collection.rs | line 1562 | 2,708 |
| src/commands/put.rs | line 1383 | 1,864 |
| src/core/db.rs | line 967 | 1,062 |

Plus split tests/collection_cli_truth.rs (108 KB) by command.

Rules:
- Public-API tests (those that only use `pub` items) move to
  tests/<area>_<feature>.rs.
- White-box tests (touching private items) STAY inline in src/. Note
  each one with `// reason: white-box; needs <private-item>`.
- Split by feature when moving, not by source file. The 6,596-line
  vault_sync mod tests should become 6–10 files
  (tests/vault_sync_ipc.rs, tests/vault_sync_restore.rs,
  tests/vault_sync_watcher.rs, tests/vault_sync_session.rs, etc.) —
  one tests/ file per concern, ≤1,500 LOC each per the §10 budgets.
- Helpers shared across the new files go into tests/common/ (existing
  pattern in this repo).
- Each source file's migration is its own commit so bisect remains
  useful. Run `cargo test` before and after each commit; numbers must
  match (or only grow).

Do NOT split the production code itself in this proposal — that is
proposals #4 and #5. The output of this work is "src/ files are smaller
and tests/ is bigger; production logic untouched."

Reference: docs/CODE_REVIEW.md §1.5, §4.2, §10.
```

---

## Proposal #4 — `decompose-vault-sync-module`

**Goal:** Decompose the 5,908-line production half of `vault_sync.rs` into a
`vault_sync/` directory with one file per concern, and split
`VaultSyncError`'s 30+-variant kitchen-sink enum into per-subsystem child
enums composed via `#[from]`. Folds in §5.3's longest-function extractions
that live in this file.

**Acceptance criteria:**

- `src/core/vault_sync.rs` becomes `src/core/vault_sync/mod.rs` plus
  ~9 sibling files matching the layout in `docs/CODE_REVIEW.md` §1.3:
  `error.rs`, `session.rs`, `ownership.rs`, `write_lock.rs`,
  `ipc/{mod,socket,handler}.rs`, `watcher.rs`, `restore.rs`,
  `recovery.rs`, `precondition.rs`.
- No new file exceeds 800 LOC.
- `VaultSyncError` becomes a thin parent enum with `#[error(transparent)]`
  variants forwarding to per-subsystem child enums (`IpcError`,
  `RestoreError`, `ConflictError`, `WatcherError`). Each child enum
  lives next to the code that produces it.
- `String`-typed debugging metadata (`missing_samples`,
  `mismatched_samples`, `extra_samples` per §2.3) becomes
  `Vec<PathBuf>` so `mcp/server.rs::map_vault_sync_error` can surface
  the data structurally.
- The 11 existing external call sites
  (`grep -rE "use crate::core::vault_sync"`) compile unchanged thanks
  to `mod.rs` re-exports. No public API change.
- The 175+-line functions `start_serve_runtime` and `begin_restore`
  (§5.3) are extracted into named phases inside their new module
  (e.g. `start_serve_runtime` → `bind_socket` + `register_session` +
  `spawn_watcher`).
- Behavior tests from #3 (`tests/vault_sync_*.rs`) continue to pass
  unchanged.

**Prompt to feed `/opsx:propose`:**

```
/opsx:propose Decompose src/core/vault_sync.rs (currently 5,908 LOC of
production code, plus another 6,596 LOC of tests already moved by
proposal #3 move-inline-tests-to-tests-dir) into a directory module
src/core/vault_sync/ following the layout proposed in
docs/CODE_REVIEW.md §1.3:

  src/core/vault_sync/
  ├── mod.rs        // re-exports + small ensure_unix_platform helpers
  ├── error.rs      // VaultSyncError parent + child error types (§2.2)
  ├── session.rs    // register/unregister/heartbeat/sweep_stale_sessions
  ├── ownership.rs  // live_collection_owner / acquire_owner_lease / release_owner_lease
  ├── write_lock.rs // with_write_slug_lock + write_dedup helpers
  ├── ipc/
  │   ├── mod.rs    // ServeRuntime, IpcSocketLocation
  │   ├── socket.rs // socket auth + permission checks (cfg(unix))
  │   └── handler.rs// handle_ipc_client / accept_ipc_clients
  ├── watcher.rs    // CollectionWatcherState, WatchEvent, WatchBatchBuffer
  ├── restore.rs    // begin_restore / finalize_pending_restore / RestoreManifest
  ├── recovery.rs   // RecoveryInProgressGuard + post-rename sentinels
  └── precondition.rs // FsPreconditionInspection + check_fs_precondition

Two structural changes ride along:

1. Split VaultSyncError per §2.2: today it's one 30+-variant enum
   spanning IPC permission, restore manifests, reconcile fences, conflict
   detection, registry poisoning. Replace with a parent enum that
   #[from]-composes child enums (IpcError, RestoreError, ConflictError,
   WatcherError, plus shared variants like Sqlite/Io). Each child enum
   lives next to its producing module. Pattern matching in callers
   becomes more focused.

2. Replace String-typed debug metadata with structured types per §2.3.
   Specifically `NewRootVerificationFailed { missing_samples: String,
   mismatched_samples: String, extra_samples: String }` becomes
   `Vec<PathBuf>` for each. Display::fmt does the join. This unblocks
   src/mcp/server.rs::map_vault_sync_error from re-parsing strings.

Plus extract the two longest-functions per §5.3 that live here:
- start_serve_runtime (227 lines) → split into bind_socket +
  register_session + spawn_watcher named phases.
- begin_restore (181 lines) → split into validate_target +
  stage_pending + register_manifest.

The 11 existing external call sites
(`grep -rE "use crate::core::vault_sync"`) compile unchanged via mod.rs
re-exports — public API does not change. Verify by running
`cargo build` after each split commit.

Hard constraints:
- No file exceeds 800 LOC after the split.
- No public API change. Re-exports in mod.rs preserve every existing
  `crate::core::vault_sync::Foo` import path.
- Behavior tests from tests/vault_sync_*.rs (added by proposal #3)
  pass before and after each commit.
- Each new file gets a one-paragraph `//!` module doc — proposal #7
  will then warn-on-missing across the public surface.

Reference: docs/CODE_REVIEW.md §1.3, §2.2, §2.3, §5.3.
```

---

## Proposal #5 — `decompose-mcp-server-module`

**Goal:** Decompose the 26-tool monolithic `impl QuaidServer` block into
domain-grouped modules, extract error and validation helpers to their own
files, and standardize error mapping across all tools.

**Acceptance criteria:**

- `src/mcp/server.rs` shrinks to just the `QuaidServer` struct, its
  `ServerHandler` impl, and the bootstrap. The 26 `#[tool]` methods
  move to `src/mcp/tools/<domain>.rs` files using multiple `impl
  QuaidServer { ... }` blocks (allowed by the `#[tool]` macro).
- Domain grouping per `docs/CODE_REVIEW.md` §1.4:
  - `tools/pages.rs`: memory_get, memory_put, memory_list, memory_raw
  - `tools/search.rs`: memory_query, memory_search
  - `tools/links.rs`: memory_link, memory_link_close, memory_backlinks,
    memory_graph
  - `tools/conversation.rs`: memory_add_turn, memory_close_session,
    memory_correct*
  - `tools/assertions.rs`: memory_check
  - `tools/tags.rs`: memory_tags, memory_timeline
  - `tools/gaps.rs`: memory_gap, memory_gaps
  - `tools/admin.rs`: memory_stats, memory_collections,
    memory_namespace_*
- `src/mcp/errors.rs` houses every `map_*_error` helper currently at
  lines 357–546. `src/mcp/validation.rs` houses every validator
  (`validate_slug`, `validate_token`, `validate_temporal_value`).
- Every tool returns errors via `mcp/errors.rs` helpers — no remaining
  ad-hoc `rmcp::Error::new(ErrorCode(-32003), …)` calls per §2.4. The
  audit at `src/mcp/server.rs:1802–1808` is fixed in this change.
- Public MCP wire surface is unchanged. All MCP integration tests pass.
- No file exceeds 800 LOC.

**Prompt to feed `/opsx:propose`:**

```
/opsx:propose Decompose src/mcp/server.rs (currently 5,903 total LOC; ~2,015
production LOC after proposal #3 moves the 3,888 LOC of tests out) into a
domain-grouped module structure per docs/CODE_REVIEW.md §1.4. The single
`impl QuaidServer { ... }` block at lines 820–1995 holds 26 #[tool]
methods. The #[tool] macro permits multiple impl blocks, so domain
grouping is a pure cut/paste with re-exports.

Target layout:

  src/mcp/
  ├── mod.rs
  ├── server.rs        // QuaidServer struct, ServerHandler impl,
  │                    //   bootstrap. Tools live elsewhere.
  ├── errors.rs        // all map_*_error helpers (currently lines 357–546)
  ├── validation.rs    // validate_slug, validate_token,
  │                    //   validate_temporal_value (currently lines 38–356)
  └── tools/
      ├── pages.rs        // memory_get, memory_put, memory_list, memory_raw
      ├── search.rs       // memory_query, memory_search
      ├── links.rs        // memory_link, memory_link_close,
      │                   //   memory_backlinks, memory_graph
      ├── conversation.rs // memory_add_turn, memory_close_session,
      │                   //   memory_correct*
      ├── assertions.rs   // memory_check
      ├── tags.rs         // memory_tags, memory_timeline
      ├── gaps.rs         // memory_gap, memory_gaps
      └── admin.rs        // memory_stats, memory_collections,
                          //   memory_namespace_*

Plus a §2.4 cleanup that rides along: audit all 26 tools for error
mapping consistency. The known offender is server.rs:1802–1808 which
uses an ad-hoc `rmcp::Error::new(ErrorCode(-32003), …)` instead of the
existing helpers. Every tool must route errors through
src/mcp/errors.rs::map_*_error helpers — no more ad-hoc construction.

Hard constraints:
- Public MCP wire surface unchanged. Every MCP client integration test
  passes before and after each commit.
- No file exceeds 800 LOC.
- The `#[tool]` macro must still register every tool. Verify by
  inspecting the generated tool list (e.g. via the MCP `tools/list`
  endpoint, or by counting #[tool] attributes after the move).
- Suggested commit sequence:
  1. Extract errors.rs (mechanical cut/paste of map_*_error).
  2. Extract validation.rs (mechanical cut/paste of validators).
  3. Audit and fix the ad-hoc error construction at server.rs:1802–1808
     (and any others surfaced by grep).
  4. Move tools by domain, one commit per tools/<domain>.rs file. Each
     commit moves the methods, leaves a `// moved to tools/<domain>.rs`
     marker for one commit, then removes the marker in the next.

Each new file gets a one-paragraph `//!` module doc — proposal #7
will then warn-on-missing.

Reference: docs/CODE_REVIEW.md §1.4, §2.4.
```

---

## Proposal #6 — `collapse-search-fn-variants`

**Goal:** Replace the 12+4 telescoping function variants in `core/fts.rs` and
`core/search.rs` with a single function each accepting a `Default`-able
parameter struct. Deletes ~200 LOC and removes the `#[allow(dead_code)]` on
unused un-namespaced variants.

**Acceptance criteria:**

- `src/core/fts.rs` exposes one `pub fn search_fts(conn: &Connection, q:
  FtsQuery<'_>) -> Result<Vec<SearchResult>, SearchError>`. The 12
  existing public variants are gone. Internal helpers may remain
  private.
- `FtsQuery` is `#[derive(Default, Clone)]`, with all filtering options
  as struct fields (`query`, `wing`, `collection`, `namespace`,
  `include_superseded`, `canonical`, `limit`).
- `src/core/search.rs` exposes one `pub fn hybrid_search(conn:
  &Connection, q: HybridSearch<'_>) -> ...`, replacing the 4 existing
  variants.
- All `#[allow(clippy::too_many_arguments)]` annotations in these two
  files are deleted (§5.2 removed by struct refactor).
- The 9 `#[allow(dead_code)]` annotations across `core/fts.rs`,
  `core/graph.rs`, `core/assertions.rs`, `core/db.rs` (§3.1) are
  audited; ones tied to un-namespaced variants are deleted with the
  variants they protect, the rest gain `// reason:` comments per #1's
  rules.
- All callers updated; no functionality regression. Tests pass.

**Prompt to feed `/opsx:propose`:**

```
/opsx:propose Collapse the telescoping function variants in
src/core/fts.rs and src/core/search.rs into single entry points
accepting a Default-able parameter struct. Currently:

- src/core/fts.rs:87–290 has 12 public functions that are progressively
  thicker shims around one private search_fts_internal:
  search_fts, search_fts_with_namespace, search_fts_canonical,
  search_fts_canonical_with_namespace,
  search_fts_canonical_with_namespace_filtered, etc. Several have
  #[allow(clippy::too_many_arguments)] to silence the 7-positional-arg
  warning. Several others are tagged #[allow(dead_code)] because nothing
  calls the un-namespaced variants any more.
- src/core/search.rs:20–105 has 4 variants of hybrid_search:
  hybrid_search, hybrid_search_with_namespace, hybrid_search_canonical,
  hybrid_search_canonical_with_namespace.

Replace both with a struct-of-options pattern from docs/CODE_REVIEW.md
§1.6:

  #[derive(Default, Clone)]
  pub struct FtsQuery<'a> {
      pub query: &'a str,
      pub wing: Option<&'a str>,
      pub collection: Option<i64>,
      pub namespace: Option<&'a str>,
      pub include_superseded: bool,
      pub canonical: bool,
      pub limit: usize,
  }

  pub fn search_fts(conn: &Connection, q: FtsQuery<'_>)
      -> Result<Vec<SearchResult>, SearchError> { … }

Same shape for HybridSearch + hybrid_search.

Required cleanup along the way:
- Delete all #[allow(clippy::too_many_arguments)] in these two files.
- Audit the 9 #[allow(dead_code)] annotations across core/fts.rs,
  core/graph.rs, core/assertions.rs, core/db.rs (the ones documented
  in §3.1). Variants tied to un-namespaced fan-outs get deleted with
  the function. Anything kept gets a `// reason:` comment per the
  conventions established in proposal #1.
- All callers updated. The struct-literal form
  `FtsQuery { query, namespace: Some(ns), ..Default::default() }` is
  the canonical call site idiom — show this in the rustdoc.

Hard constraints:
- No behavior change. Each test that currently exercises one of the 12
  variants exercises the same query through the new struct. Diff the
  test output before and after to confirm.
- ~200 LOC net deletion expected per the original review.
- Independent of all other proposals. Can run in parallel with
  proposals #4 and #5.

Reference: docs/CODE_REVIEW.md §1.6, §5.1, §5.2, §3.1 (dead_code
allowances).
```

---

## Proposal #7 — `add-public-api-rustdoc`

**Goal:** Add crate-level `//!` documentation, module-level docs at the top
of every `core/*.rs`, and turn on `#![warn(missing_docs)]` on the public
surface so future drift is caught at compile time.

**Acceptance criteria:**

- `src/lib.rs` (currently 3 lines) gains a multi-paragraph `//!` doc
  describing Quaid: what it is, how its modules fit together, where to
  start reading.
- Every file under `src/core/` and `src/mcp/` (post-split) starts with
  a `//!` module-level doc describing its single responsibility.
- `#![warn(missing_docs)]` lands on `src/lib.rs`. Every `pub fn`,
  `pub struct`, `pub enum`, `pub trait` in `src/core/` and `src/mcp/`
  has a `///` doc comment with at least one sentence.
- `cargo doc --no-deps` builds cleanly with no warnings.
- The doc comment style follows the existing good examples (e.g.
  `core/fts.rs`'s `/// Expands a sanitized multi-token query into an
  explicit FTS5 OR chain.`).

**Prompt to feed `/opsx:propose`:**

```
/opsx:propose Add crate-level and public-API documentation to the
Quaid codebase. After proposals #4 (split-vault-sync) and #5
(split-mcp-server) land, the module structure is stable enough to
document. Today src/lib.rs is 3 lines, vault_sync.rs has almost no doc
comments, and there is no crate-level //! intro — per docs/CODE_REVIEW.md
§6.1 this is the first thing rustdoc readers hit and currently a blank.

Scope:

1. src/lib.rs gains a multi-paragraph //! crate doc that explains:
   - What Quaid is (one-paragraph elevator from CLAUDE.md is fine to
     adapt, but write fresh — CLAUDE.md is for agents, lib.rs //! is for
     human and rustdoc readers).
   - The module map: core (lib internals: db, search, embeddings,
     conversation, vault_sync, …), mcp (server + tools), commands (CLI
     dispatch).
   - Where to start reading. For consumers of the library:
     mcp/server.rs and core/conversation. For maintainers:
     core/db.rs and core/vault_sync/mod.rs.

2. Every file under src/core/ and src/mcp/ (post #4/#5 split) starts
   with a //! module doc. One paragraph, focused on the single
   responsibility, plus a one-line "see also" pointing at adjacent
   modules. Example skeleton:
   //! Conversation extraction queue: SQLite-backed pending/running
   //! state, lease-expiry recovery, and per-session collapse semantics
   //! for debounced and forced enqueues.
   //!
   //! See also: [`super::extractor`] for job processing,
   //! [`super::turn_writer`] for the producer side.

3. #![warn(missing_docs)] lands on src/lib.rs. Every pub fn / pub
   struct / pub enum / pub trait in src/core/ and src/mcp/ gets a ///
   doc comment with at least one sentence. Existing good examples to
   match in style:
   - src/core/fts.rs: "/// Expands a sanitized multi-token query into
     an explicit FTS5 OR chain."
   - src/core/conversation/queue.rs: the new doc comments added in
     fix-extraction-force-correctness on `enqueue` and
     `enqueue_force_path`.

4. cargo doc --no-deps must build with zero warnings after this lands.

Don't document private items or `commands/` (binary entry points) —
those are not part of the public crate surface. The CLI's own help
text covers commands/.

This proposal is best done after #4 and #5 because documenting a
12 KLOC monolith is wasted; documenting nine 600-line modules with
clear single responsibilities is tractable.

Reference: docs/CODE_REVIEW.md §6.1, §6.2.
```

---

## Notes on running this plan

- Each proposal is independent enough to be a single PR. Treat the
  prompts above as the "user request" for `/opsx:propose` — paste,
  review the generated artifacts, then `/opsx:apply`.
- Proposals #1, #3, and #6 can ship in any order; the rest have the
  dependencies shown in the sequencing diagram.
- After each proposal archives, return here and tick the row in the
  Quick reference table. Once all seven are archived, delete this file.
- If any proposal balloons mid-implementation (e.g. #4 surfaces
  unexpected coupling), pause and split rather than expanding scope.
