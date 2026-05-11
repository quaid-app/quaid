## 1. vault_sync.rs:3380 — collapse to let-else

- [x] 1.1 In `src/core/vault_sync.rs::finalize_pending_restore`, replace lines 3372–3380 with a single `let Some(pending_root_path) = collection.pending_root_path.as_deref() else { ... };` block whose `else` arm preserves the existing two outcomes (`OrphanRecovered` when `state == CollectionState::Restoring`, `NoPendingWork` otherwise). Keep the subsequent `PathBuf::from(pending_root_path)` conversion (now applied to `&str`, no `clone().unwrap()`).
- [x] 1.2 Verify by `cargo build` that no other call sites depended on `collection.pending_root_path.unwrap()` shape from this region of the file.
- [x] 1.3 Add regression test `finalize_pending_restore_returns_no_pending_work_when_pending_root_path_is_null` to the existing `mod tests` in `src/core/vault_sync.rs`. The test SHALL: (a) open an in-memory db with the production schema, (b) insert a `collections` row with `pending_root_path = NULL` and `state = 'created'` (or whatever non-Restoring state the helpers expose), (c) call `finalize_pending_restore(&conn, collection_id, FinalizeCaller::Watcher)`, (d) assert `Ok(FinalizeOutcome::NoPendingWork)`.
- [x] 1.4 Add regression test `finalize_pending_restore_returns_orphan_recovered_when_restoring_with_null_pending_root` mirroring 1.3 but with `state = 'restoring'` and asserting `Ok(FinalizeOutcome::OrphanRecovered)`. Confirm the row's state is reverted by `revert_orphan_restore_state` — read the row back and assert `state` is no longer `Restoring`.
- [x] 1.5 Run the targeted test: `cargo test --lib finalize_pending_restore_returns`. Both new tests must pass.

## 2. mcp/server.rs — `serialize_response` helper + four call-site rewrites

- [x] 2.1 Add a private helper `fn serialize_response<T: serde::Serialize>(value: &T) -> Result<String, rmcp::Error>` near the existing `fn map_anyhow_error` (around `src/mcp/server.rs:400`). Body: `serde_json::to_string_pretty(value).map_err(|e| map_anyhow_error(anyhow::Error::from(e)))`.
- [x] 2.2 Rewrite `src/mcp/server.rs:1793` from `serde_json::to_string_pretty(&result).unwrap()` to `serialize_response(&result)?`. Confirm the surrounding `Ok(CallToolResult::success(vec![Content::text(...)]))` still type-checks.
- [x] 2.3 Rewrite `src/mcp/server.rs:1880` (also `&result`) to `serialize_response(&result)?`.
- [x] 2.4 Rewrite `src/mcp/server.rs:1892` (`&collections`) to `serialize_response(&collections)?`.
- [x] 2.5 Rewrite `src/mcp/server.rs:1990` (`&result`) to `serialize_response(&result)?`.
- [x] 2.6 If any of 2.2–2.5 fails to compile due to a trait-bound issue (e.g. the local binding type does not implement `serde::Serialize` directly), fall back to inline `.map_err(|e| map_anyhow_error(anyhow::Error::from(e)))?` at that single site and note it in a code comment that names the bound mismatch. Do not block the other three sites on it.
- [x] 2.7 Add regression test `serialize_response_returns_rmcp_error_on_unrepresentable_input` to the existing `mod tests` in `src/mcp/server.rs`. The test SHALL feed `serialize_response` a `serde_json::Value` containing an `f64::NAN` field (which `serde_json` refuses to serialize) and assert `.is_err()` with no panic. The test must not require a running MCP server — it exercises the helper in isolation.
- [x] 2.8 Run targeted tests: `cargo test --lib serialize_response_returns_rmcp_error_on_unrepresentable_input` and `cargo test --lib mcp::server`. New test passes; existing tests still pass.

## 3. inference.rs — recover from mutex poisoning on six sites

- [x] 3.1 Convert `src/core/inference.rs:259` from `.lock().expect("model runtime lock poisoned")` to `.lock().unwrap_or_else(|e| e.into_inner())`. Function: `configure_runtime_model`.
- [x] 3.2 Convert `src/core/inference.rs:272` (in `runtime_model_config`) the same way.
- [x] 3.3 Convert `src/core/inference.rs:1034` (in `ensure_model`, read pre-check) the same way.
- [x] 3.4 Convert `src/core/inference.rs:1044` (in `ensure_model`, write install) the same way.
- [x] 3.5 Convert `src/core/inference.rs:1065` (in `embed`) the same way.
- [x] 3.6 Convert `src/core/inference.rs:1079` (in `embedding_evidence_kind`) the same way.
- [x] 3.7 Re-grep `src/core/inference.rs` for any remaining `.lock().expect(` or `.lock().unwrap(` in production (non-`#[cfg(test)]`) code. If any survived, document why or convert. Test-only call sites (e.g. line 1574, 1579, 1977) remain unchanged.
- [x] 3.8 Add regression test `embed_recovers_from_poisoned_model_runtime_mutex` to the existing `mod tests` in `src/core/inference.rs`. The test SHALL: (a) acquire the existing `env_mutation_lock()` and set `QUAID_FORCE_HASH_SHIM = "1"` (mirroring the existing test at line 1576), (b) spawn a thread that does `let _g = model_runtime().lock().unwrap(); panic!("intentional");` and `.join()` it, capturing the panic, (c) call `configure_runtime_model(default_model())` and `embed("text")` and assert both return `Ok(_)`, (d) reset state via `model_runtime().lock().unwrap_or_else(|e| e.into_inner()).loaded = None;` to avoid bleeding into other tests.
- [x] 3.9 Run targeted test: `cargo test --lib embed_recovers_from_poisoned_model_runtime_mutex`. Test passes without panic.

## 4. Verification against the lint gate

- [x] 4.1 Run `cargo clippy --lib --tests -- -D clippy::unwrap_used -D clippy::expect_used` (or whatever the precise invocation in `add-rust-lints-and-ci-gate` is once it lands, scoped to production code only) and confirm no production `unwrap_used` or `expect_used` warnings remain in `vault_sync.rs`, `mcp/server.rs`, or `inference.rs` from the sites enumerated in `docs/CODE_REVIEW.md` §2.1.
- [x] 4.2 Run the full test suite: `cargo test`. All pre-existing tests continue to pass; the three new regression tests pass.
- [x] 4.3 Run `cargo build --release` (default `embedded-model` features) to confirm no release-build regressions.

## 5. Documentation

- [x] 5.1 Update `docs/CODE_REVIEW.md` §2.1 by adding a "Resolution status" line for each of the three bullet points, naming this OpenSpec change (`remove-production-panic-paths`) and the regression test that locks the fix in. Mirror the format already used by the §1 amendment block for `fix-extraction-force-correctness`.
- [x] 5.2 No CLAUDE.md updates required — no new architectural surface, no new MCP tools, no new CLI flags.
