## Why

Proposal #1 (`add-rust-lints-and-ci-gate`) wires a deny-`unwrap_used` / deny-`expect_used` clippy gate into CI and immediately surfaces 11 production panic paths flagged in `docs/CODE_REVIEW.md` §2.1: one in `core/vault_sync.rs::finalize_pending_restore`, four in MCP tool handlers, and six in the embedding-runtime hot path. The lint gate cannot land green until these sites stop panicking, and each panic is a narrow correctness bug in its own right (mid-restore state regression, MCP server crash on JSON serialization, daemon crash on mutex poisoning).

## What Changes

- Convert `src/core/vault_sync.rs:3380`'s `.unwrap()` on `collection.pending_root_path` into a `let Some(...) = ... else { ... }` that preserves the existing `OrphanRecovered` / `NoPendingWork` outcomes and removes the now-redundant `is_none()` check on lines 3372–3378.
- Add a `serialize_response<T: Serialize>(value: &T) -> Result<String, rmcp::Error>` helper to `src/mcp/server.rs` that funnels through `map_anyhow_error`. Route the four `serde_json::to_string_pretty(...).unwrap()` sites at `src/mcp/server.rs:1793, 1880, 1892, 1990` through it and propagate via `?`.
- Convert the six `lock().expect("...")` calls in `src/core/inference.rs` (lines 259, 272, 1034, 1044, 1065, 1079) to the recover-from-poison pattern `lock().unwrap_or_else(|e| e.into_inner())` already in use at `src/mcp/server.rs:1804`.
- Add a regression test per fix:
  - vault_sync: a unit test that constructs a collection row with `pending_root_path = NULL` in each relevant `state` and asserts `finalize_pending_restore` returns the expected `FinalizeOutcome` without panicking.
  - mcp/server: a unit test against `serialize_response` that feeds it a value `serde_json` refuses to serialize (e.g. a `f64::NAN` field) and asserts a structured `rmcp::Error` is returned (not a panic).
  - inference: a unit test that poisons `MODEL_RUNTIME` from a panicking thread and then invokes `embed` (or `configure_runtime_model`) and asserts it succeeds rather than re-panicking on poison.

No new error variants are introduced — `VaultSyncError::InvariantViolation` already exists at `src/core/vault_sync.rs:442` and is used opportunistically only if the let-else investigation surfaces a state we cannot collapse into the existing outcome arms. No CLI surface, MCP tool surface, on-disk format, schema, or migration changes.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `vault-sync`: codifies that `finalize_pending_restore` is total over all reachable `(state, pending_root_path)` combinations and never panics on missing-but-expected fields mid-restore.
- `slm-runtime`: extends the existing "SLM panic isolation does not crash the daemon" contract to cover poisoned `MODEL_RUNTIME` mutex acquisition on the embedding hot path. Embedding is invoked synchronously by MCP tool handlers, where a `catch_unwind` boundary does not exist, so poison-on-acquire SHALL be recovered in-process.

## Impact

- Code: `src/core/vault_sync.rs` (one branch restructure in `finalize_pending_restore`), `src/mcp/server.rs` (one new helper, four call-site rewrites), `src/core/inference.rs` (six lock-acquisition rewrites).
- Tests: three new regression tests as enumerated above. The vault_sync test goes in the existing `mod tests` block in `src/core/vault_sync.rs`; the mcp test in `src/mcp/server.rs`; the inference test in `src/core/inference.rs`.
- Behavior: `finalize_pending_restore` is no longer reachable in a panicking state; MCP tools return JSON-RPC errors instead of crashing the daemon on (impossible-but-now-handled) serialization failures; embedding survives a poisoned model-runtime mutex by recovering the inner guard.
- Lints: this change is the prerequisite that lets `add-rust-lints-and-ci-gate`'s `unwrap_used` / `expect_used` denies compile clean. After this lands, that proposal's CI gate flips green without further intervention.
- Risk: low. The vault_sync change preserves existing outcome semantics (verified by reading the call graph above the unwrap); the MCP helper only refactors infallible-input call sites; the inference change matches an already-deployed pattern in the same crate.
- Out of scope: §2.2 error-type splitting, §2.3 string-typed error metadata, §2.4 MCP error-mapping audit. Those remain follow-up work per the user-stated scope ("remove panics surfaced by lints").
