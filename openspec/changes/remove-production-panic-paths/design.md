## Context

Proposal #1 (`add-rust-lints-and-ci-gate`) introduces `[lints.clippy] unwrap_used = "deny"`, `expect_used = "deny"`, and a CI gate that fails on any production occurrence. `docs/CODE_REVIEW.md` §2.1 enumerates the production unwrap/expect sites that survive after `cfg(test)` filtering: 1 in `core/vault_sync.rs`, 4 in `mcp/server.rs`, 6 in `core/inference.rs`. Every one is an in-process `panic!` reachable from operator-driven code paths (the MCP daemon, `quaid extract`, `quaid serve`).

The three sites share a common pattern — "infallible in practice, panicking on principle" — but the right fix differs by site:

- The `vault_sync.rs:3380` unwrap is *almost* infallible: lines 3372–3378 already early-return when `pending_root_path.is_none()`. The unwrap is unreachable in current call graphs but the compiler does not know it; a future refactor that removes the explicit `is_none()` guard would silently regress to a panic.
- The four `mcp/server.rs` unwraps wrap `serde_json::to_string_pretty` over `serde_json::json!{...}` literals (no `f64::NAN`, no custom serializers), so they are infallible *for the inputs constructed in the same function*. They are still wrong on principle: a copy-paste into a future tool that serializes a `f64`-bearing struct would panic the entire MCP daemon.
- The six `inference.rs` `lock().expect("...")` calls panic on `PoisonError` — i.e. when another thread held the lock and unwound. The same crate already uses the recovery pattern `lock().unwrap_or_else(|e| e.into_inner())` at `src/mcp/server.rs:1804` for the same reason: in a long-running daemon, a poisoned mutex is recoverable as long as the protected state is rebuildable, and the embedding-runtime state (`ModelRuntime { configured, loaded }`) is rebuildable on the next call by re-running `ensure_model`.

This change removes all 11 panic paths without changing observable contracts, so `add-rust-lints-and-ci-gate`'s deny lints land green.

## Goals / Non-Goals

**Goals:**

- Eliminate every production `unwrap` / `expect` listed in `docs/CODE_REVIEW.md` §2.1 (11 sites total).
- Each fix carries a regression test against a path the failure mode is actually reachable on, so a future refactor that reintroduces the panic also reintroduces a failing test.
- Preserve every observable contract — same outcomes, same error codes, same JSON shapes, same lock-acquisition order, no schema or CLI changes.
- After this change lands, `add-rust-lints-and-ci-gate`'s `unwrap_used = "deny"` and `expect_used = "deny"` clippy gates compile clean against the production crate (test code remains exempt).

**Non-Goals:**

- §2.2 (`VaultSyncError` kitchen-sink split into per-subsystem error enums). Out of scope per user instruction.
- §2.3 (string-typed error variants → `Vec<PathBuf>`). Out of scope.
- §2.4 (MCP error-mapping convention audit across all 26 tools). Out of scope; we only touch the four `to_string_pretty` sites.
- §1 file-size monoliths. Out of scope.
- Adding new variants to `VaultSyncError`. The existing `InvariantViolation { message: String }` variant at `src/core/vault_sync.rs:442` is the only error surface this change uses, and only as a fallback if the let-else collapse cannot preserve the existing outcomes (it can — see Decision 1).
- Changing the `expect_used` / `unwrap_used` lint configuration itself. That is proposal #1's surface; this change merely makes that configuration land green.

## Decisions

### 1. `vault_sync.rs:3380` — collapse the duplicate `is_none()` guard into a `let-else` that preserves existing outcomes

**Decision:** Replace the current shape

```rust
if collection.pending_root_path.is_none() {
    if collection.state == CollectionState::Restoring {
        revert_orphan_restore_state(conn, &collection)?;
        return Ok(FinalizeOutcome::OrphanRecovered);
    }
    return Ok(FinalizeOutcome::NoPendingWork);
}

let pending_root_path = PathBuf::from(collection.pending_root_path.clone().unwrap());
```

with

```rust
let Some(pending_root_path) = collection.pending_root_path.as_deref() else {
    if collection.state == CollectionState::Restoring {
        revert_orphan_restore_state(conn, &collection)?;
        return Ok(FinalizeOutcome::OrphanRecovered);
    }
    return Ok(FinalizeOutcome::NoPendingWork);
};
let pending_root_path = PathBuf::from(pending_root_path);
```

This collapses the two branches that previously formed an "if-then-unwrap" into a single structurally-enforced match. The compiler now refuses to compile a future change that drops the explicit guard and reintroduces a panicking unwrap.

**Alternatives considered:**

- *Add a new `VaultSyncError::PendingRestoreMissingRoot` variant and return it.* Rejected: the existing call graph through lines 3372–3378 already converts the missing-root state into a structured outcome (`NoPendingWork` or `OrphanRecovered`). Replacing that with an error would be a behavior change in the error-channel direction, not a panic-removal — and the proposal scope is "remove panics, do not change observable contracts". A new variant would also fold into §2.2's kitchen-sink critique, which is explicitly out of scope.
- *Reuse the existing `VaultSyncError::InvariantViolation { message }`.* Rejected for the same reason: the call graph proves this state is *not* an invariant violation; it is a valid mid-restore state that the function already handles deliberately. Returning `InvariantViolation` from a non-invariant-violating state would mislead callers and pollute logs.
- *Keep the two-branch shape, replace `.unwrap()` with `.expect("guarded above")`.* Rejected: clippy `expect_used = "deny"` rejects this for the same reason as `unwrap_used`. We would only have moved the panic site one line to the right.

**Why the let-else collapse:** it is the smallest possible diff that satisfies both the lint and the user's "preserve invariants enforced by the call graph" intent. The behavior is byte-equivalent — the same outcome is returned in the same conditions — and the structural simplification is a strict win.

### 2. `mcp/server.rs` — single `serialize_response` helper routed through `map_anyhow_error`

**Decision:** Add the following private helper near the existing error-mapping helpers (around line 400):

```rust
fn serialize_response<T: serde::Serialize>(value: &T) -> Result<String, rmcp::Error> {
    serde_json::to_string_pretty(value).map_err(|e| map_anyhow_error(anyhow::Error::from(e)))
}
```

Then rewrite the four call sites at lines 1793, 1880, 1892, 1990 from:

```rust
Ok(CallToolResult::success(vec![Content::text(
    serde_json::to_string_pretty(&result).unwrap(),
)]))
```

to:

```rust
Ok(CallToolResult::success(vec![Content::text(
    serialize_response(&result)?,
)]))
```

`anyhow::Error: From<E>` for any `E: std::error::Error + Send + Sync + 'static`, so `serde_json::Error` converts cleanly. `map_anyhow_error` already maps unknown errors into the daemon's preferred error code (`-32003`), so the four sites get consistent JSON-RPC error mapping for free.

**Alternatives considered:**

- *Inline `.map_err(|e| rmcp::Error::new(ErrorCode(-32003), e.to_string(), None))?` at each of the four sites.* This is the existing pattern at e.g. `src/mcp/server.rs:1810-1811`. Rejected as the *first* fix — the helper version is one line shorter at each call site (a strict refactor win) and keeps error-code policy in one place. The inline form is acceptable as a fallback if `serialize_response` runs into trait-bound issues with the actual `serde_json::json!` return type.
- *Make `serialize_response` panic on error with a `tracing::error!` logged first.* Rejected: that defeats the entire point of removing the panic path.
- *Audit all `.map_err(...)` sites in `server.rs` and unify them.* That is §2.4 in the code review, explicitly out of scope.

**Why this exact helper:** the user's instructions name this helper directly ("once a `serialize_response` helper exists"). It is the smallest possible new surface, scoped to one file, removing four panic sites in four lines of caller diff each.

### 3. `inference.rs` — recover from `PoisonError` on six hot-path mutex acquisitions

**Decision:** Convert each of these six sites from `.lock().expect("model runtime lock poisoned")` (or equivalent) to `.lock().unwrap_or_else(|e| e.into_inner())`:

| Line | Function | Lock |
|------|----------|------|
| 259 | `configure_runtime_model` | `MODEL_RUNTIME` |
| 272 | `runtime_model_config` | `MODEL_RUNTIME` |
| 1034 | `ensure_model` (read pre-check) | `MODEL_RUNTIME` |
| 1044 | `ensure_model` (write install) | `MODEL_RUNTIME` |
| 1065 | `embed` | `MODEL_RUNTIME` |
| 1079 | `embedding_evidence_kind` | `MODEL_RUNTIME` |

The pattern is the same one already in production at `src/mcp/server.rs:1804`. Recovering the inner `MutexGuard` is safe for `MODEL_RUNTIME` because:

- `ModelRuntime` is `{ configured: ModelConfig, loaded: Option<EmbeddingModel> }`. Neither field requires invariants beyond "is the configured model the same as the loaded model", which `ensure_model` re-checks on every call and reloads if not.
- Any panic mid-mutation that *did* corrupt `loaded` to a half-built state would, on recovery, be observed by the next `ensure_model` invocation (which compares `loaded.config` against `configured` and reloads on mismatch). At worst the daemon performs one redundant model load. At best (and in practice) the panicking code did not partially mutate `loaded` because the assignments are atomic on the Rust language level.
- Embedding is invoked synchronously by MCP tool handlers (`memory_search`, `memory_put`, etc.) and by the extraction worker. None of those callers run inside the `catch_unwind` boundary that `slm-runtime`'s "SLM panic isolation" requirement establishes — that boundary covers SLM *inference* panics inside extraction, not embedding mutex acquisition. So a poisoned-mutex panic *would* tear down the entire `quaid serve` process, which is the failure we are preventing.

**Alternatives considered:**

- *Replace `Mutex` with a `parking_lot::Mutex` (no poisoning).* Adds a dependency that proposal #1's lints flag as unjustified. Out of scope for a panic-removal change.
- *Wrap embedding in `catch_unwind` as `slm-runtime` does for SLM inference.* Solves a different problem (panics inside the model code) and is much heavier than warranted for fixing six lock acquisitions. The lock-poisoning recovery is the precise, surgical fix.
- *Use `.lock().map_err(InferenceError::from)?` and propagate the poison error to the caller.* Would require changing `embed`/`configure_runtime_model` signatures (`configure_runtime_model` returns `()`) and is a larger surface change than the user authorized. Recovery via `into_inner()` is the documented best-practice for this exact situation.

**Why poison-recovery is defensible here, but not universally:** if `MODEL_RUNTIME` held data that *cannot* be safely observed after a panic (e.g. invariants between two fields that are mutated non-atomically), recovery would mask a real bug. That is not the case here: the only invariant is `loaded.config == configured`, and it is re-validated on every entry to `ensure_model`. This is the same line of reasoning that justifies the existing pattern at `server.rs:1804`.

### 4. Regression test strategy — one test per fix, exercising the actual failure mode

**Decision:** Three new tests in three existing `mod tests` blocks. Each test deliberately constructs the state that *was* the panic trigger and asserts a non-panicking outcome.

**(a) `vault_sync.rs` — `finalize_pending_restore_returns_no_pending_work_when_pending_root_path_is_null`:**

Insert a `collections` row with `pending_root_path = NULL` (and no other pending state) using the test helpers already present in `mod tests`. Call `finalize_pending_restore(&conn, collection_id, FinalizeCaller::Watcher)` and assert it returns `Ok(FinalizeOutcome::NoPendingWork)`. Repeat with `state = 'restoring'` and assert `Ok(FinalizeOutcome::OrphanRecovered)`. The pre-fix code does not panic here either (the explicit guard handles it), but the test pins the *outcome* so a future refactor that drops the guard cannot regress to either a panic *or* an incorrect outcome.

The test does not need a `debug_assert!` because the call graph proves the let-else covers every reachable state; the test asserts that cover-age directly.

**(b) `mcp/server.rs` — `serialize_response_returns_rmcp_error_on_unrepresentable_input`:**

Construct a `serde_json::Value` containing a key whose value is `f64::NAN` (or use `serde::Serializer` with a custom type that returns `Err`). Call `serialize_response(&value)` and assert it returns an `Err(rmcp::Error)` (specifically, the variant produced by `map_anyhow_error` for an unknown error string). The four production call sites pass JSON-safe inputs, but the helper's contract is "errors do not panic", so the test exercises the helper's own contract.

**(c) `inference.rs` — `embed_recovers_from_poisoned_model_runtime_mutex`:**

Spawn a thread that acquires `model_runtime().lock()` and panics inside the guard, deliberately poisoning the mutex. Join the thread (capturing the panic). Then call `configure_runtime_model(default_model())` followed by `embed("text")` (under the hash-shim env-var guard already used in the existing tests at line 1577) and assert both succeed. The test must reset the runtime via `model_runtime().lock().unwrap_or_else(|e| e.into_inner()).loaded = None;` afterward to avoid bleeding state into other tests in the same process (mirroring line 1579's existing pattern, but using the new poison-recovery path).

The hash shim avoids a real model download; the test's purpose is to exercise the *lock-acquisition* path, not the inference path.

## Risks / Trade-offs

- **[Risk] `serialize_response` helper introduces `anyhow::Error::from(serde_json::Error)` conversion at every call site, which adds a cheap allocation on the success path.** → Mitigation: the success path is `Ok(...)` from `to_string_pretty`, which never enters the `map_err` branch, so no allocation occurs in practice. The error path allocates one `anyhow::Error`, but that path is unreachable for the four call sites with their current inputs.
- **[Risk] Recovering from a poisoned `MODEL_RUNTIME` could mask a real bug where another thread panicked while mutating `loaded`.** → Mitigation: `ensure_model` revalidates `loaded.config == configured` on every call and reloads on mismatch. The worst-case outcome is one redundant model load per recovered poisoning event. This is the precedent already set by `src/mcp/server.rs:1804`.
- **[Risk] The vault_sync regression test as specified depends on test helpers in `mod tests` that may not currently expose direct insertion of `collections` rows with `pending_root_path = NULL`.** → Mitigation: if no helper exists, write the row via raw `conn.execute(...)` — `mod tests` already has direct SQL helpers for collection setup (used by other restore-path tests). The test scope is small enough to inline the setup.
- **[Trade-off] We do not add `VaultSyncError::PendingRestoreMissingRoot` even though some readers might prefer an explicit error variant for the "should never happen" case.** → Decision: explicit out-of-scope per the user's instruction to not fold structural error-type changes into a panic-removal proposal. If the let-else collapse is found to be insufficient during implementation (e.g. discovers a third reachable state), the fallback is to use the existing `InvariantViolation { message }` variant rather than minting a new one.

## Migration Plan

No migration. This is internal-correctness only: no schema, no on-disk format, no CLI/MCP surface, no config keys.

Rollback: revert the commit. None of the new code is observable from outside the binary.

## Open Questions

- The mcp helper is the user's preferred form, but we should verify during implementation that all four sites pass `serde_json::Value` (not a `&serde_json::Map<...>` or other type that would require trait-bound gymnastics in `serialize_response`). Spot-check during implementation; fall back to inline `.map_err(|e| map_anyhow_error(anyhow::Error::from(e)))?` at any site where the helper's `T: Serialize` bound is awkward.
- The inference test relies on poisoning a `static` mutex; if test isolation requires it, we can wrap the test logic in a fresh `Mutex<ModelRuntime>` rather than mutating the global. Decide during implementation based on test-runner parallelism.
