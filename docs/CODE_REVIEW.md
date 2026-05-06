# Quaid Rust Code Review

**Reviewer:** Senior code reviewer, Rust focus
**Date:** 2026-05-06
**Branch reviewed:** `fix/issue-162-wireup-worker` @ `152f5a0`
**Reference:** Apollo GraphQL [Rust Best Practices Handbook](https://github.com/apollographql/rust-best-practices)

---

## Amendment — 2026-05-06 (post Codex adversarial review)

A second-pass adversarial review (Codex job `b95cg1cgh`) and an independent
verification against the actual code surfaced three correctness issues in the
extraction path that the original review below missed. Two of them are real
bugs in code this branch directly touches, and they outrank the structural
recommendations as ship-blockers. The original review's prioritization is
amended accordingly: the file-size and lints recommendations stand, but they
are **follow-up work**, not P0.

The fixes are tracked in OpenSpec change `fix-extraction-force-correctness`
(`openspec/changes/fix-extraction-force-correctness/`).

### Top blockers (added)

- **[high] `extract --force` only re-extracts the latest day-file** —
  `src/commands/extract.rs:67-80` resets every day-file's cursor to `0` but
  enqueues a single job for `target.latest_relative_path`.
  `extractor::process_job` (`src/core/conversation/extractor.rs:257-293`)
  processes exactly one `conversation_path` per job, so older day-files in
  multi-day sessions stay reset at cursor `0` and are never re-extracted.
  Contradicts the documented `--force` intent.
- **[high] Cursor reset rewrites conversation markdown without the session
  lock** — `src/commands/extract.rs:151-159` does `format::parse` →
  in-memory mutate → `fs::write`. `turn_writer::append_turn`
  (`src/core/conversation/turn_writer.rs:75-81, 155-160`) acquires both an
  in-process per-session mutex and an on-disk `SessionFileLock` before any
  mutation. A concurrent `memory_add_turn` between `reset_cursors`'s parse
  and write loses the appended turn. Realistic scenario: MCP server running
  while the operator runs `quaid extract <id> --force`.
- **[medium] Queue transaction wrapper does not roll back on `COMMIT`
  failure** — `src/core/conversation/queue.rs:307-321`'s
  `with_immediate_transaction` only runs `ROLLBACK` on closure error. On a
  failed `COMMIT` (notably `SQLITE_BUSY`, which does not auto-rollback)
  the connection is left inside an open transaction. Because the queue
  uses a shared `Connection`, follow-up writes fail with nested-transaction
  errors. Practical risk is narrow (write lock is already held by
  `BEGIN IMMEDIATE`), but the fix is strictly better with no downside —
  use `rusqlite::Transaction` with RAII `Drop` rollback.

### Reprioritization

| Severity | Item | Status |
|---|---|---|
| Blocker | `extract --force` rebuild gap | tracked in `fix-extraction-force-correctness` |
| Blocker | Unlocked cursor-reset rewrite | tracked in `fix-extraction-force-correctness` |
| Should-fix | Queue commit-failure recovery | tracked in `fix-extraction-force-correctness` |
| Follow-up | File-size monoliths (§1) | unchanged from original; not a ship-gate |
| Follow-up | Error-type splitting (§2.2–2.3) | unchanged |
| Follow-up | Lints + CI gate (§3) | unchanged; valuable but not blocking |
| Follow-up | API/test/docs (§4–§7) | unchanged |

### What the original review got right

The strengths called out in §9 (`thiserror` discipline, optimistic
concurrency, conversation sub-module organization, unsafe-free codebase,
test naming, no `TODO`/`FIXME` rot) all hold. The structural critiques in
§1–§7 are still valid — they are simply not what blocks shipping this
branch.

---

## TL;DR (original — superseded by the amendment above)

Quaid's core engineering is solid: `unwrap`/`expect` is almost entirely confined to `#[cfg(test)]` blocks, error types use `thiserror` with structured fields, and the conversation sub‑module already demonstrates the splitting pattern the rest of the codebase needs to adopt. The single biggest health problem is file size: **eight modules exceed 1,800 lines**, the worst (`core/vault_sync.rs`) is **12,504 lines**. This is corrosive to both human review and agent context windows, and most of it is mechanical to fix because production logic is already cleanly factored into 50–200 line functions — they just live in one mega‑file alongside their tests.

The recommendations below are grouped by priority. The "concrete refactor plan" at the end is sequenced so each step is a self‑contained PR.

> **Note (post-amendment):** The "P0" framing for file-size monoliths in §1
> is superseded. See the amendment above. §1 is reclassified as follow-up.

---

## 1. File‑size monoliths (P0 — biggest single win)

### 1.1 The numbers

| File | Total | Prod | Tests | Notes |
|---|---:|---:|---:|---|
| `src/core/vault_sync.rs` | **12,504** | 5,908 | 6,596 | IPC + watchers + restore + reconcile + sessions |
| `src/core/reconciler.rs` | **7,403** | 3,118 | 4,285 | Fresh‑attach, full‑hash, stat‑diff, rename detection |
| `src/mcp/server.rs` | **5,903** | 2,015 | 3,888 | 26 MCP tools in one `impl` block |
| `src/commands/collection.rs` | **4,269** | 1,561 | 2,708 | `collection {add,list,info,sync,restore,audit,ignore,quarantine}` |
| `src/commands/put.rs` | **3,246** | 1,382 | 1,864 | `put` + IPC client + CLI/serve dispatch |
| `src/core/db.rs` | **2,028** | 966 | 1,062 | Schema bootstrap + migrations + config |
| `src/core/inference.rs` | **2,025** | 1,433 | 592 | Embedding model load + candle backends |
| `src/core/quarantine.rs` | **1,865** | 1,269 | 596 | Quarantine export/restore/discard |
| `src/core/conversation/model_lifecycle.rs` | 1,484 | 1,328 | 156 | Online model download/cache |

### 1.2 Why this is worth fixing first

- **Agents (and humans) re‑read whole files.** A single grep into `vault_sync.rs` costs ~458 KB of context. Splitting it into `vault_sync/{error,session,ipc,watcher,restore,write_lock,…}.rs` lets Claude (and reviewers) load only the file they need.
- **`mod tests { … }` at the bottom doubles file length for free.** ~50 % of every monolith is its own test module. Moving these to `tests/<name>.rs` integration tests, or to a sibling `<file>_tests.rs` brought in via `#[cfg(test)] mod tests;`, halves the file length with zero behavioural change.
- **Compile times.** `cargo build`'s incremental cost is per‑file; a 12 K‑line file is one big CGU after edits.

### 1.3 Recommendation: split `vault_sync.rs` first

`src/core/vault_sync.rs` is the largest file and also the one with the most distinct concerns. It already has natural seams (each `cfg(unix)` block, each runtime‑registry helper). Suggested split:

```
src/core/vault_sync/
├── mod.rs                  // re-exports + the small "ensure_unix_platform" helpers
├── error.rs                // VaultSyncError (currently lines 328–579, ~250 lines)
├── session.rs              // register/unregister/heartbeat/sweep_stale_sessions (1983–2046)
├── ownership.rs            // live_collection_owner / acquire_owner_lease / release_owner_lease (2048–2263)
├── write_lock.rs           // with_write_slug_lock + write_dedup helpers (1561–1977)
├── ipc/
│   ├── mod.rs              // ServeRuntime, IpcSocketLocation
│   ├── socket.rs           // socket auth + permission checks (cfg(unix) heavy)
│   └── handler.rs          // handle_ipc_client / accept_ipc_clients
├── watcher.rs              // CollectionWatcherState, WatchEvent, WatchBatchBuffer
├── restore.rs              // begin_restore / finalize_pending_restore / RestoreManifest
├── recovery.rs             // RecoveryInProgressGuard + post-rename sentinels
└── precondition.rs         // FsPreconditionInspection + check_fs_precondition
```

With only **11 call‑sites** outside the module (`grep -rE "use crate::core::vault_sync"` returns 11 lines, mostly importing `VaultSyncError` and `ResolvedSlug`), the public surface is small and the refactor is a re‑export job. After the split, each new file lands in the 300–800 LOC range — well within human/agent comfort.

### 1.4 Recommendation: split `mcp/server.rs` by tool group

The single `impl QuaidServer { … }` block at `src/mcp/server.rs:820–1995` holds **26** `#[tool]` methods. The `#[tool]` macro doesn't require them to live in one `impl` — Rust allows multiple `impl` blocks. Group by domain:

```
src/mcp/
├── mod.rs
├── server.rs               // QuaidServer struct, ServerHandler impl, tests
├── errors.rs               // map_db_error / map_search_error / map_vault_sync_error / … (38 lines each, currently 357–546)
├── validation.rs           // validate_slug / validate_token / validate_temporal_value (38–356)
└── tools/
    ├── pages.rs            // memory_get, memory_put, memory_list, memory_raw
    ├── search.rs           // memory_query, memory_search
    ├── links.rs            // memory_link, memory_link_close, memory_backlinks, memory_graph
    ├── conversation.rs     // memory_add_turn, memory_close_session, memory_correct*
    ├── assertions.rs       // memory_check
    ├── tags.rs             // memory_tags, memory_timeline
    ├── gaps.rs             // memory_gap, memory_gaps
    └── admin.rs            // memory_stats, memory_collections, memory_namespace_*
```

The 200‑line block of `map_*_error` adapters at lines 357–546 is also a fine first extraction on its own — pure mechanical cut/paste, no behaviour change.

### 1.5 Recommendation: move integration‑style tests out of `src/`

For each of the eight monoliths above, a substantial fraction of total lines is a single bottom‑of‑file `mod tests { … }`. **The `tests/` directory is already heavily used (40 files there).** Move tests that exercise public APIs out:

| File | `mod tests` location | Action |
|---|---|---|
| `vault_sync.rs:5909` | 6,596 lines | move to `tests/vault_sync_*.rs` (split by concern when moving) |
| `reconciler.rs:3119` | 4,285 lines | move to `tests/reconciler_*.rs` |
| `server.rs:2016` | 3,888 lines | move to `tests/mcp_server_*.rs` |
| `collection.rs:1562` | 2,708 lines | move to `tests/cli_collection_*.rs` |
| `put.rs:1383` | 1,864 lines | move to `tests/cli_put_*.rs` |
| `db.rs:967` | 1,062 lines | move to `tests/db_*.rs` |

Inline tests should be reserved for white‑box tests that need access to private items. Anything that calls only public APIs belongs in `tests/`.

### 1.6 Recommendation: collapse the search/FTS API surface

`src/core/fts.rs:87–169` has 12 public functions that are progressively thicker shims around one private `search_fts_internal`. The same pattern exists in `src/core/search.rs:20–105` (`hybrid_search`, `hybrid_search_with_namespace`, `hybrid_search_canonical`, `hybrid_search_canonical_with_namespace`). Several are tagged `#[allow(dead_code)]` because nothing calls the un‑namespaced variants any more.

Replace the 12‑function fan‑out with a single struct:

```rust
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

pub fn search_fts(conn: &Connection, q: FtsQuery<'_>) -> Result<Vec<SearchResult>, SearchError> { … }
```

Callers gain readability (`FtsQuery { query, namespace: Some(ns), ..Default::default() }`), the dead‑code `#[allow]` annotations disappear, and a future flag (e.g. `min_score`) doesn't add yet another `_filtered` variant.

---

## 2. Error handling (P1 — mostly good, one structural issue)

### 2.1 Production `unwrap`/`expect` is genuinely rare

Counted only outside `mod tests`:

| File | Prod `unwrap`/`expect` | Verdict |
|---|---:|---|
| `src/core/vault_sync.rs` | 4 | 3 of 4 are `cfg(test)` gated; **only `:3380`** is real risk (see below) |
| `src/mcp/server.rs` | 4 | All `serde_json::to_string_pretty(...).unwrap()` — safe but should be `?` |
| `src/core/reconciler.rs` | 0 production unwraps |
| `src/commands/put.rs` | 0 production unwraps (8 are `#[cfg(all(test, unix))]`) |
| `src/commands/collection.rs` | 0 production unwraps |
| `src/core/db.rs` | 4 | All in test/utility helpers |
| `src/core/inference.rs` | 6 | Mostly mutex `lock().unwrap()`; convert to `unwrap_or_else(|e| e.into_inner())` per existing pattern at `server.rs:1804` |

This is significantly cleaner than typical Rust codebases. Apollo's chapter 4 calls out `panic!` and `unwrap` as the #1 thing to avoid in libraries — Quaid is already there.

**Concrete fixes:**

- `src/core/vault_sync.rs:3380` — `let pending_root_path = PathBuf::from(collection.pending_root_path.clone().unwrap());` will panic if `pending_root_path` is `None` while a restore is mid‑flight. The two lines above already check `manifest_json.is_some()`. Use `let Some(pending_root_path) = collection.pending_root_path.as_deref() else { return Ok(FinalizeOutcome::NoPendingWork); };` or return `VaultSyncError::InvariantViolation { … }`.
- `src/mcp/server.rs:1793, 1880, 1892, 1990` — `serde_json::to_string_pretty(...).unwrap()` is technically infallible for these inputs, but propagating with `.map_err(map_anyhow_error)?` (or a dedicated `serialize_response()` helper) removes the panic path and is one line shorter than `.unwrap()` once you factor out the helper. There are likely more in non‑checked code paths; clippy will find them.

### 2.2 `VaultSyncError` is a kitchen sink

`src/core/vault_sync.rs:328–579` is one enum with **30+ variants** spanning IPC permission, restore manifests, reconcile fences, conflict detection, and registry poisoning. Apollo chapter 4 ("Defining Errors") recommends per‑subsystem error types composed via `#[from]`:

```rust
// vault_sync/error.rs
pub enum VaultSyncError {
    #[error(transparent)] Ipc(#[from] IpcError),
    #[error(transparent)] Restore(#[from] RestoreError),
    #[error(transparent)] Conflict(#[from] ConflictError),
    #[error(transparent)] Watcher(#[from] WatcherError),
    #[error(transparent)] Sqlite(#[from] rusqlite::Error),
    // … shared across all
}
```

Each child enum lives next to the code that produces it. Pattern matching in callers becomes more focused (`if let Err(VaultSyncError::Conflict(ConflictError::HashMismatch { … })) = …`) and extending one subsystem doesn't force a recompile of every caller of every other subsystem's enum.

### 2.3 String‑typed metadata in error variants

Several variants stuff debugging context into `String` fields with `Display` formatting that the caller then has to re‑parse:

```rust
NewRootVerificationFailed {
    missing_samples: String,    // "samples/a, samples/b"
    mismatched_samples: String, // pre-formatted by Display::fmt
    extra_samples: String,
}
```

Prefer `Vec<PathBuf>` and let `Display` do the join. Otherwise tools that consume errors (your own `mcp/server.rs:421:map_vault_sync_error`) can never structurally surface the data.

### 2.4 `Result` consistency in MCP

`src/mcp/server.rs:1802–1808` uses an ad‑hoc `rmcp::Error::new(ErrorCode(-32003), …)` instead of the existing `map_db_error` helper. Audit all 26 tools for a single error‑mapping convention; the helpers at `:357–546` are nearly there.

---

## 3. Linting & compiler gates (P1 — cheap, prevents regressions)

### 3.1 No `[lints]` table, no crate‑level deny attributes

`Cargo.toml` has no `[lints.rust]` or `[lints.clippy]` table. `src/main.rs` and `src/lib.rs` have no `#![deny(...)]` / `#![warn(...)]` attributes. There is no `clippy.toml` or `rustfmt.toml`.

Apollo chapter 2 recommends:

```toml
# Cargo.toml
[lints.rust]
unsafe_code = "forbid"        # you have none — lock it in
missing_docs = "warn"         # for the future
unreachable_pub = "warn"

[lints.clippy]
# Apollo's defaults (start strict, downgrade as needed)
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
# The four that actually prevent bugs:
unwrap_used = "warn"
expect_used = "warn"
panic = "warn"
print_stdout = "warn"         # use tracing/eprintln, never println in libs
# Plus the perf trio:
redundant_clone = "warn"
needless_collect = "warn"
large_enum_variant = "warn"
```

This single change would surface:
- The four `serde_json::*.unwrap()` cases in `mcp/server.rs`.
- 6 mutex `lock().unwrap()` cases in `core/inference.rs`.
- The `large_enum_variant` issue in `VaultSyncError` (variants like `NewRootVerificationFailed` are ~64 bytes; the enum is bloated to that size for every error).
- The 9 `#[allow(dead_code)]` annotations in `core/fts.rs`, `core/graph.rs`, `core/assertions.rs`, `core/db.rs`, etc.

### 3.2 Replace `#[allow]` with `#[expect]`

The codebase uses 46 `#[allow(…)]` annotations (`grep -rE '#\[allow\('`) and only 2 `#[expect(…)]`. As of Rust 1.81, prefer `#[expect(…)]`: it warns when the suppressed lint *no longer fires*, catching dead suppressions. Each suppression should also have a one‑line `// reason: ...` comment per Apollo chapter 2.

### 3.3 CI gate

Add to your CI workflow (likely `.github/workflows/`):

```yaml
- run: cargo clippy --all-targets --all-features --locked -- -D warnings
- run: cargo fmt --all -- --check
```

Without this, the lints above can be added but will silently bit‑rot.

---

## 4. Testing patterns (P2 — quality is fine, organisation is the issue)

### 4.1 What's good

- Tests use descriptive names: `process_job_should_not_partially_advance_cursor_when_later_window_fails` (`tests/extraction_worker.rs`), `extraction_enable_leaves_flag_false_when_integrity_check_fails` (`tests/cli_extraction.rs`), `restore_cleans_up_tempfile_when_write_fails`. This matches Apollo chapter 5 verbatim.
- Test fixtures are organised under `tests/fixtures/` and `tests/common/`.
- Snapshot testing is already in use (`watcher_core.rs`, `model_lifecycle.rs`, `quarantine_revision_fixes.rs`).

### 4.2 What needs work

- **Tests living inside production source.** See §1.5 — `vault_sync.rs`, `reconciler.rs`, `server.rs`, etc. each have multi‑thousand‑line `mod tests` blocks that are pulling their parent files past readable size.
- **Multiple assertions per test.** Spot‑checking `tests/extraction_worker.rs`, several tests have 5+ `assert!`/`assert_eq!` calls. Apollo chapter 5 recommends one logical assertion per test; the test name then becomes unambiguous when it fails. Where a single test must check several invariants, prefer custom assertion helpers (e.g., `assert_extraction_unchanged(&before, &after)`) so the failure message names *what* was expected.
- **`tests/collection_cli_truth.rs` is 108,492 bytes.** The same anti‑pattern as the source‑side monoliths. Split by command (`tests/cli_collection_add.rs`, `tests/cli_collection_sync.rs`, …).

---

## 5. API patterns

### 5.1 Telescoping function variants

Already covered in §1.6. Same smell appears in:
- `core/search.rs:20–105` — 4 variants of `hybrid_search`
- `core/fts.rs:87–290` — 12 variants of `search_fts`
- `core/graph.rs` (verify) — likely similar

Prefer one function with a `Default`-able params struct.

### 5.2 Long parameter lists

`src/core/fts.rs:150` — `search_fts_canonical_with_namespace_filtered(query, wing_filter, collection_filter, namespace_filter, include_superseded, conn, limit)` — 7 positional args, easy to mis‑order at the call site. Two `#[allow(clippy::too_many_arguments)]` are already in this file. The params‑struct approach in §1.6 fixes this for free.

### 5.3 Function length

The longest 5 production functions in the codebase:

| Lines | File | Function |
|---:|---|---|
| 302 | `commands/put.rs:733` | `persist_with_vault_write` |
| 227 | `core/vault_sync.rs:227` | `start_serve_runtime` |
| 193 | `core/reconciler.rs:???` | `apply_reingest` |
| 181 | `core/vault_sync.rs:???` | `begin_restore` |
| 175 | `core/vault_sync.rs:???` | `exercise_writer_side_sentinel_crash_core` (test helper) |

`persist_with_vault_write` deserves to be broken up into named phases (`stage_pending`, `acquire_dedup_key`, `commit_or_recover`, …). 300‑line functions are very hard to step through; with current tests you have the freedom to extract.

### 5.4 `main.rs` `match cli.command`

`src/main.rs:309` is a 147‑line `async fn main` with an N‑arm `match`. Each arm is already a one‑liner that calls `commands::<x>::run(...)`. Two cleanups:

1. Move the matcher into `fn dispatch(db, cli) -> Result<()>` so `main` is just bootstrap.
2. Several arms are doing argument plumbing (e.g., `namespace.as_deref().or(Some(""))` is repeated). Push that defaulting into the command modules so the dispatch becomes mechanical.

---

## 6. Documentation

### 6.1 Public‑API docs are sparse

`src/lib.rs` is three lines. Crate‑level docs (`//! …`) are missing — Apollo chapter 8 calls this out as the first thing rustdoc readers hit.

For internal functions, doc comments are inconsistent: `core/fts.rs` has good ones (`/// Expands a sanitized multi-token query into an explicit FTS5 OR chain.`), `core/vault_sync.rs` has almost none. Recommend `#![warn(missing_docs)]` on `lib.rs` once the modules stabilise — at minimum on every `pub fn` in `core/`.

### 6.2 Comments

Spot check: comments in this codebase are largely *good* — they explain why, not what (e.g., `vault_sync.rs:74–94` constants block, `vault_sync.rs:3370–3385` finalize ordering). No `TODO`/`FIXME` markers in source at all (`grep` returns 0). Maintain this.

---

## 7. Performance & idioms

These are all minor; the hot path was clearly profiled at some point.

- `core/inference.rs:567` `mean_pool_and_normalize` — should be inlined and the temporary `Vec<f32>` allocations checked. Use `cargo flamegraph --bench extraction` (you have a bench harness) before optimising.
- Several `clone()` calls in the MCP layer (`server.rs:822` etc.) on `String` slugs that could be `&str`. Worth one clippy‑driven sweep; `clippy::redundant_clone` will find them.
- `core/vault_sync.rs` instantiates `Sha256` in several places — fine, it's per‑request, not in a hot loop.

Not a current problem, but worth a tracking note: when the codebase grows, watch for `large_enum_variant` on `VaultSyncError`. The enum is currently 64–96 bytes per variant because of the longest variant; every `Result<T, VaultSyncError>` pays for that. Boxing rare large variants (`Box<NewRootVerificationFailed>`) keeps the enum compact.

---

## 8. Concrete refactor plan (suggested PR sequence)

Each step is independent, ships its own tests, and shrinks the worst monolith without behaviour changes.

| # | Scope | Files touched | Approx LOC delta |
|--:|---|---|---:|
| 1 | Add `[lints.rust]` + `[lints.clippy]` to `Cargo.toml`; add CI clippy gate | `Cargo.toml`, `.github/workflows/*.yml` | +30 |
| 2 | Move `mod tests` blocks out of `vault_sync.rs`, `reconciler.rs`, `server.rs`, `db.rs`, `collection.rs`, `put.rs` into `tests/<name>_*.rs` integration files | 6 prod files, 12+ new test files | -16,000 from `src/`, +16,000 to `tests/` |
| 3 | Split `core/vault_sync.rs` into `core/vault_sync/` sub‑module per §1.3 | `vault_sync.rs` → 8–10 files | net 0 |
| 4 | Extract `mcp/server.rs` `map_*_error` helpers to `mcp/errors.rs`; extract validators to `mcp/validation.rs` | `server.rs` → +2 files | net 0 |
| 5 | Split MCP tool methods by domain into `mcp/tools/*.rs` per §1.4 | `server.rs` → 8 files | net 0 |
| 6 | Split `core/reconciler.rs` along the `apply_*_rename`, `walk_*`, `full_hash_*` axes | `reconciler.rs` → 5–6 files | net 0 |
| 7 | Replace search/FTS function explosion with `FtsQuery` / `HybridSearch` params structs | `core/fts.rs`, `core/search.rs`, all callers | -200 LOC |
| 8 | Split `VaultSyncError` into per‑subsystem child enums composed via `#[from]` | `core/vault_sync/error.rs`, callers | net 0 |
| 9 | Crate‑level `//!` docs in `lib.rs`; module‑level docs at top of each `core/*.rs`; `#![warn(missing_docs)]` on the public surface | 30+ files | +500 |
| 10 | Break up the 5 longest production functions (§5.3) | 3 files | net 0 |

After step 2 alone, **no production source file exceeds ~6,000 lines**. After step 3, no file exceeds ~1,500. After steps 4–6, no file exceeds ~800.

---

## 9. What's already excellent — keep doing it

- `thiserror` everywhere, `anyhow` only in binary entry points (`commands/*.rs` use `anyhow::Result`, libraries use typed errors). Exactly what Apollo chapter 4 prescribes.
- Optimistic concurrency in `memory_put` with `expected_version` is the right pattern; the conflict variant in `VaultSyncError` is structured.
- The `core/conversation/` sub‑module is the role model for how the rest of `core/` should be organised (12 files, none over 1,500 lines, clear single responsibility per file).
- Unsafe‑free codebase. Run `unsafe_code = "forbid"` to lock it in.
- Test names already follow `subject_should_outcome_when_condition` pattern.
- Zero `TODO`/`FIXME` rot.

---

## 10. Quick‑reference: file‑size budgets to hold the line

Once the refactor lands, suggest informal budgets enforced in code review:

- **Production module:** target ≤ 800 lines, hard ceiling 1,500 lines.
- **Test module (`tests/*.rs`):** target ≤ 1,500 lines (split by feature, not by file under test).
- **Function:** target ≤ 80 lines, hard ceiling 200.
- **`impl` block:** target ≤ 400 lines, split into multiple `impl` blocks if larger.

A single `cargo xtask line-budget` script or a CI step counting `wc -l src/**/*.rs | awk '$1>1500'` keeps this honest cheaply.
