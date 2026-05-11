## ADDED Requirements

### Requirement: Embedding-runtime mutex acquisition recovers from poisoning instead of panicking

Every acquisition of the `MODEL_RUNTIME` mutex in `core::inference` (covering `configure_runtime_model`, `runtime_model_config`, `ensure_model`, `embed`, `embedding_evidence_kind`, and any future hot-path consumer of the same mutex) SHALL recover from `std::sync::PoisonError` by extracting the inner guard via `into_inner()` rather than panicking. This is the same recovery pattern already in production at `mcp::server::QuaidServer::memory_gaps` (`src/mcp/server.rs:1804`) and is justified for `MODEL_RUNTIME` because (a) the only invariant on the protected state is `loaded.config == configured`, which `ensure_model` revalidates on every entry, and (b) embedding is invoked synchronously from MCP tool handlers and the extraction worker — neither runs inside the `catch_unwind` boundary that protects SLM inference, so a poisoned-mutex panic would tear down the entire `quaid serve` process.

This requirement extends, but does not modify, the existing "SLM panic isolation does not crash the daemon" requirement: SLM inference itself remains isolated by `catch_unwind`; embedding additionally tolerates poison-on-acquire because it has no `catch_unwind` boundary around it.

#### Scenario: `embed` succeeds after another thread poisoned the model-runtime mutex

- **WHEN** a thread acquires `MODEL_RUNTIME.lock()` and panics inside the guard, leaving the mutex poisoned, and subsequently another thread calls `embed("text")` after `ensure_model()` is allowed to run
- **THEN** the call returns a normal `Ok(Vec<f32>)` (or a non-panic `InferenceError` if the model is genuinely unloaded), `quaid serve` does not abort, and the next embedding call observes a usable `MODEL_RUNTIME`

#### Scenario: `configure_runtime_model` succeeds against a poisoned runtime

- **WHEN** `MODEL_RUNTIME` is poisoned and `configure_runtime_model(model)` is called
- **THEN** the call returns normally, the runtime's `configured` field is updated, and `loaded` is invalidated as before, all without re-panicking on the poison

#### Scenario: Recovery does not silently mask a real corruption

- **WHEN** any consumer of `MODEL_RUNTIME` recovers a poisoned guard via `into_inner()`
- **THEN** the next call into `ensure_model` revalidates `loaded.config == configured` and reloads the model on mismatch; consumers do not assume the recovered state is correct without that revalidation
