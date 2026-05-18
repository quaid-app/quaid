## 1. SLM dtype and diagnostics

- [ ] 1.1 Add `SlmRuntimeInfo` and lightweight dtype-summary types in the conversation SLM module, including alias, model path, device, selected dtype, source dtype summary, estimated weight bytes, loaded state, runtime-disabled state, and fallback reason.
- [ ] 1.2 Implement a header-only safetensor dtype scanner that reads the safetensor header length and JSON header without reading tensor payload bytes.
- [ ] 1.3 Replace the hard-coded `DType::F32` SLM VarBuilder load with the memory-aware selection rules from `runtime-memory-footprint`.
- [ ] 1.4 Add one-shot F32 fallback when lower-precision Candle model construction fails, and persist the fallback reason in `SlmRuntimeInfo`.
- [ ] 1.5 Add organized tests under `tests/` for BF16, F16, F32, mixed dtype, malformed header, and fallback diagnostic behavior using tiny synthetic safetensor files.

## 2. Shared SLM runtime ownership

- [ ] 2.1 Add a production shared SLM accessor backed by `OnceLock<Arc<LazySlmRunner>>`.
- [ ] 2.2 Update `LazySlmState` to store the loaded alias and reload by dropping the prior runner when a different alias is requested.
- [ ] 2.3 Update `QuaidServer::new` and HTTP/SSE service construction to use the shared production SLM while preserving `QuaidServer::new_with_slm` for tests.
- [ ] 2.4 Update the extraction worker construction to use the same shared production SLM runtime as MCP correction.
- [ ] 2.5 Add tests proving two production server instances and the extraction worker path use the same shared runtime handle without loading duplicate model state.

## 3. Registry cleanup

- [ ] 3.1 Update `with_write_slug_lock` so uncontended per-target lock entries are removed after the guarded action completes.
- [ ] 3.2 Preserve write-lock safety under contention by removing entries only when the registry still points to the same `Arc` and no external waiters/clones remain.
- [ ] 3.3 Add idle-tracker pruning for closed, already-closed, missing, or stale sessions during idle scans.
- [ ] 3.4 Add organized tests under `tests/` for uncontended write-lock eviction, contended write-lock retention, closed idle-session cleanup, and stale idle-session pruning.

## 4. Status and operator diagnostics

- [ ] 4.1 Update `quaid extraction status` to display loaded/not-loaded/runtime-disabled state, selected dtype, source dtype summary, device, estimated weight bytes, and fallback reason when available.
- [ ] 4.2 Remove or replace the hard-coded `phi-3.5-mini` `~2.0 GiB` estimate with metadata-derived estimates or `unknown`.
- [ ] 4.3 Update model/extraction status tests to assert that Phi memory is not under-reported and F32 fallback is visible.

## 5. Verification

- [ ] 5.1 Run the focused runtime-memory tests added for this change.
- [ ] 5.2 Run `cargo test`.
- [ ] 5.3 Run `cargo clippy --all-targets --all-features --locked -- -D warnings`.
- [ ] 5.4 Run `openspec validate reduce-runtime-memory-footprint --strict` and resolve any proposal/spec/task validation errors.
