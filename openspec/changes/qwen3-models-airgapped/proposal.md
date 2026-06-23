## Why

Quaid's default models — BGE-small-en-v1.5 (embedding) and Phi-3.5 Mini (extraction) — are not competitive when benchmarked against other memory tools. We are switching both defaults to the Qwen3 family: `Qwen3-Embedding-0.6B` (1024d) for retrieval and `Qwen3-4B-Instruct-2507` (q4_K_M GGUF) for fact extraction, both run in-process via candle. Neither model can be `include_bytes!`'d into the binary (~1.2 GB and ~2.5 GB respectively), which forces a second, overdue change: "airgapped" must stop meaning "weights baked into the binary" and start meaning "local-only inference, with a one-time model download permitted." Today the manual model download is too cumbersome; airgapped installs should fetch and hook up their default models automatically.

## What Changes

- **BREAKING — default embedding model → `Qwen3-Embedding-0.6B`** (HF `Qwen/Qwen3-Embedding-0.6B`), candle-native. Fixed **1024 dims**, L2-normalized, reusing the existing `page_embeddings_vec_1024` table. It is a Qwen3 decoder with **last-token pooling** (not CLS like BGE) and an instruction-aware query format (`Instruct: {task}\nQuery: {query}`), so `inference.rs` gains a new pooling/query path beside the existing BGE path. `EMBEDDER_VERSION` is bumped to force a full re-embed/reindex.
- **BREAKING — default extraction model → `Qwen3-4B-Instruct-2507` at q4_K_M GGUF**, run **in-process** via candle's `quantized_qwen3` module (no Ollama, no daemon). Greedy/temp-0 decode, context capped at **8K** (must be set explicitly — the model's 262K trained context would otherwise OOM the KV cache). Strict JSON stays **best-effort** (greedy → `serde_json` parse → existing recovery), not a hard grammar guarantee. A new `SlmClient` impl is added; the Phi-3 `SlmRunner` is not mutated.
- **BREAKING — redefine "Airgapped"**: decouple the **privacy promise** (local-only inference, no cloud, no API keys, no data egress) from the **packaging mechanism** (embedded weights). Airgapped now permits a one-time, user-triggered automatic download that fetches and hooks up the default models. Collapse the `embedded-model`/`online-model` two-channel split into a single download-on-first-use channel.
- **Dependency bump**: `candle-core`/`-nn`/`-transformers` `0.8.4` → `>=0.9.2` (prefer `0.10.x`) for `quantized_qwen3` + `Q4_K`/`Q6_K` k-quant support. The `BertModel`/`XLMRobertaModel` embedding APIs are byte-stable across the bump.
- Both defaults remain user-substitutable (embedding via `QUAID_MODEL` / `--model`; extraction via its model alias/repo resolution).
- Update README, `docs/`, `docs/spec.md`, and `CLAUDE.md` wherever they describe airgapped as "BGE-small embedded" or the two-channel model.

Pre-release: breaking changes are acceptable and **no existing-database migration is provided** — re-init is expected.

## Capabilities

### New Capabilities
- `airgapped-model-provisioning`: The redefined "airgapped" contract — local-only inference decoupled from embedded packaging — plus automatic one-time fetch-and-hook-up of the default embedding and extraction models on first use, replacing the manual download.

### Modified Capabilities
- `model-selection`: Default embedding model, dimension (384→1024), pooling strategy (CLS→last-token), query-instruction format, and the "airgapped channel behaviour" sub-section all change.
- `slm-runtime`: Default in-process SLM changes from safetensors Phi-3.5 Mini to GGUF `Qwen3-4B-Instruct-2507` (q4_K_M) loaded via `quantized_qwen3`; adds the explicit 8K context cap; retains best-effort JSON, panic isolation, and lazy-load semantics.
- `brain-config-schema`: `quaid init` default `model_id` and `embedding_dim` (384→1024) change.
- `dual-release-assets`: Airgapped assets no longer embed a model bundle; the "channel choice does not require re-embedding" guarantee is removed; channels collapse to one download-on-first-use asset.
- `dual-release-install-surface`: Installer channel semantics change — no offline-embedded default asset; the single asset downloads default models on first use.
- `dual-release-validation`: Release gate no longer exercises two channel feature sets; validation covers the single download-on-first-use channel and the model-provisioning path.

## Impact

- **Code**: `src/core/inference.rs` (new Qwen3 last-token pooling + instruction query, `EMBEDDER_VERSION`, default model), `src/core/conversation/slm.rs` + new `slm_gguf.rs` (`SlmClient` impl over `quantized_qwen3`/GGUF), `src/core/conversation/model_lifecycle.rs` (single-`.gguf` fetch/verify; repoint phi3-only `model_type` gate and runner-supported-alias guards), `src/core/conversation/extractor.rs` / `src/mcp/server.rs` (runner wiring), `src/commands/model.rs`, `src/core/db.rs` / `src/schema.sql` (config defaults).
- **Dependencies**: `candle-*` 0.8.4 → ≥0.9.2; add wildcard arms to exhaustive `candle_core::DType` matches (candle adds enum variants in patch releases).
- **Cargo features / release**: collapse `embedded-model` vs `online-model`; update cross-compile/release workflow and installer for darwin-arm64/x86_64 and linux-x86_64/aarch64-musl. Single static, pure-Rust CPU binary preserved.
- **Models / licensing**: Qwen3 weights are apache-2.0; candle is MIT/Apache — both MIT-compatible. Daemon footprint drops from ~8 GB (F16 Phi-3.5) to ~2.5–3 GB (q4_K_M).
- **Docs**: README, `docs/getting-started.md`, `docs/contributing.md`, `docs/spec.md`, `docs/roadmap_*`, `CLAUDE.md`.
- **Risks**: the exact `Qwen3-4B-Instruct-2507-Q4_K_M.gguf` is not yet end-to-end load-tested (arch fit confirmed, weights not) → smoke-test task; candle decode throughput must be benchmarked (generic ops, no fused kernel); `n_ctx` must be set explicitly to bound the KV cache.
