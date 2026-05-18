## Context

Quaid currently has two long-lived SLM entry points: the extraction worker in `quaid serve` and MCP correction tools. Both eventually call `LazySlmRunner`, but production construction is not shared consistently. The extraction worker builds a fresh runner when its thread starts, while each HTTP/SSE connection creates a fresh `QuaidServer`, and `QuaidServer::new` creates a fresh `LazySlmRunner`. A client that opens multiple SSE sessions and invokes correction can therefore load multiple copies of the same model in one process.

The Phi-3.5 Mini cache is several GiB on disk, but `SlmRunner::load_inner` currently passes `DType::F32` to `VarBuilder::from_mmaped_safetensors`. For BF16/F16 safetensor files this expands resident weight storage instead of preserving the model's lower-precision representation. The extraction status estimate also reports `phi-3.5-mini` as roughly 2 GiB, which is not credible for F32-loaded weights.

Two process-global registries also need cleanup. `IDLE_TRACKERS` already removes entries on successful close, but it should expose a deterministic prune path for stale sessions and error paths. `slug_writes` currently stores an `Arc<Mutex<()>>` for every unique write target and never removes the entry after the guarded write completes.

## Goals / Non-Goals

**Goals:**

- Ensure one Quaid process can hold at most one loaded extraction/correction SLM runner per active alias.
- Load SLM weights with memory-aware dtype selection, preserving BF16/F16 when the cached model and Candle backend support it.
- Make SLM load diagnostics expose alias, dtype, device, loaded state, runtime-disabled state, and realistic resident-memory estimates.
- Remove or bound normal-operation growth in idle session and write-lock registries.
- Keep Rust code idiomatic: small focused helpers, explicit error variants, no production `unwrap`/`expect`, and tests under organized test modules/files.

**Non-Goals:**

- Replace sqlite-vec, change vector search storage, or redesign graph expansion.
- Introduce quantized GGUF model loading.
- Add new network downloads, model aliases, or external dependencies.
- Change MCP tool schemas or require users to migrate SQLite data.

## Decisions

### D1: Process-wide shared SLM runtime

Add a production shared-runtime accessor in the conversation SLM module, backed by `OnceLock<Arc<LazySlmRunner>>`. `QuaidServer::new`, the HTTP/SSE service provider, and the extraction worker shall use this accessor. Tests that need isolation shall continue using `QuaidServer::new_with_slm` or `Worker::new` with a stub client.

`LazySlmState` shall store the loaded alias alongside the runner. If a later call requests a different alias, the runner is dropped and the new alias is loaded under the same mutex. This preserves the "one loaded SLM per process" bound while avoiding stale alias reuse.

Alternatives considered:

- Per-server runners: rejected because HTTP/SSE sessions can duplicate multi-GiB weights.
- One runner per transport: rejected because extraction and correction still duplicate weights.
- Global mutable singleton without test injection: rejected because existing test seams are useful and should remain.

### D2: Memory-aware dtype selection

Replace hard-coded `DType::F32` in SLM loading with a helper that inspects safetensor headers without reading tensor payloads. The helper shall read the 8-byte safetensor header length, parse only the JSON header with `serde_json`, and derive the set of floating-point storage dtypes across all weight shards.

Selection rules:

- If all floating weight tensors are BF16, load with `DType::BF16`.
- If all floating weight tensors are F16, load with `DType::F16`.
- If all floating weight tensors are F32, load with `DType::F32`.
- If floating dtypes are mixed or unsupported, fall back to `DType::F32` and record a diagnostic reason.
- If lower-precision model construction fails with a Candle error, retry once with `DType::F32`, record that fallback reason, and expose it through status/debug state.

`SlmRunner` shall store a small `SlmRuntimeInfo` value containing alias, model directory, device, selected dtype, source dtype summary, estimated weight bytes, and fallback reason if any. This is returned by the lazy runner for status output.

Alternatives considered:

- Trust the model alias table for dtype: rejected because custom/full model IDs can have different safetensor dtypes.
- Use `safetensors::SafeTensors::deserialize` over file bytes: rejected because it requires a whole-file buffer unless combined with mmap; the header-only parser is simpler and dependency-free.
- Always force F16/BF16: rejected because mixed/custom models and backend errors need a reliable fallback.

### D3: Registry cleanup without broad refactors

For `slug_writes`, remove the map entry after the guarded action completes when the entry still points to the same `Arc` and `Arc::strong_count(&lock) == 2` while holding the registry mutex. That count represents the map entry plus the local clone; if another thread has cloned it or is waiting, leave the entry in place. The guarded action shall still run outside the registry mutex.

For `IDLE_TRACKERS`, keep existing `clear_session` behavior and add a prune helper used by idle scanning to remove db/session entries that are no longer active, already closed, or older than a conservative multiple of `extraction.idle_close_ms`. This addresses error/stale paths without changing normal close behavior.

Alternatives considered:

- Replace both registries with an LRU dependency: rejected because no external dependency is needed.
- Periodic cleanup thread: rejected because cleanup can happen at existing scan/write boundaries.
- Leave `IDLE_TRACKERS` alone: rejected because the status investigation showed this area is easy to misread; explicit bounded behavior and tests make the contract clear.

### D4: Status output reports facts, not optimistic estimates

Update extraction/model status helpers so human-readable output and any JSON status fields report:

- configured alias,
- loaded/not-loaded/runtime-disabled state,
- selected dtype and device when loaded,
- source dtype summary when available,
- estimated resident weight bytes from dtype and tensor element counts,
- fallback-to-F32 reason when applicable.

The old `~2.0 GiB` Phi estimate shall be removed or corrected. If exact tensor metadata is unavailable, status shall say `unknown` rather than under-reporting.

Alternatives considered:

- Keep static alias estimates: rejected because they were the source of misleading diagnostics.
- Add OS RSS sampling: rejected for this change because platform-specific process accounting is orthogonal to fixing duplicate loads and dtype expansion.

## Risks / Trade-offs

| Risk | Mitigation |
|------|------------|
| Lower-precision load exposes Candle backend gaps | Retry once with F32 and surface the fallback reason in status. |
| Global shared runner makes tests order-dependent | Keep injection seams and avoid using the production singleton in unit tests that need controlled state. |
| Alias changes require dropping a loaded runner | Store the loaded alias in `LazySlmState` and reload only when requested alias changes. |
| Write-lock cleanup races with waiters | Remove only when the registry still points to the same `Arc` and no external clones exist. |
| Status estimates are mistaken for exact RSS | Label values as weight estimates and expose dtype/device, not total process memory. |

## Migration Plan

No SQLite migration is required. Rollout is code-only:

1. Add SLM runtime info/dtype helpers and update SLM loading.
2. Introduce the shared production runner accessor and wire server/extraction entry points to it.
3. Add registry cleanup helpers and tests.
4. Update status output/tests.
5. Run focused integration tests, then `cargo test` and `cargo clippy --all-targets --all-features --locked -- -D warnings`.
