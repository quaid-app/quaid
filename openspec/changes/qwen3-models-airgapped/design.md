## Context

Quaid runs two model subsystems, both candle-native (pure-Rust, CPU, single static binary):

- **Embedding** (`src/core/inference.rs`): `BertModel`/`XLMRobertaModel`, default BGE-small-en-v1.5 (384d, CLS pooling). Two cargo channels — `embedded-model` (`include_bytes!` the weights, "airgapped") and `online-model` (download on first use).
- **Extraction SLM** (`src/core/conversation/slm.rs`): candle `Phi3Model` from safetensors, default Phi-3.5 Mini, greedy decode, best-effort JSON parse + recovery. Downloaded + SHA-verified via `model_lifecycle.rs`. Consumers depend only on the open `SlmClient` trait (`extractor.rs:78`), injected as `Arc<dyn SlmClient>`.

Both defaults are uncompetitive on memory benchmarks. We switch to the Qwen3 family. The new models are far too large to embed (~1.2 GB / ~2.5 GB), so the "airgapped == embedded bytes" packaging model breaks and must be redefined. A background research sweep (candle in-tree vs mistral.rs vs llama-cpp-2 vs the pure-Rust long tail), with adversarial verification of the front-runners, backs the decisions below.

Constraints: single static, pure-Rust CPU binary; cross-compile to darwin-arm64/x86_64 and linux-x86_64/aarch64-musl; MIT-compatible; no external daemon. Pre-release — breaking changes OK, no DB migration required.

## Goals / Non-Goals

**Goals:**
- Default embedder → `Qwen3-Embedding-0.6B` (1024d, last-token pooling, instruction-aware queries), in-process via candle.
- Default extractor → `Qwen3-4B-Instruct-2507` q4_K_M GGUF, in-process via candle `quantized_qwen3`, greedy/temp-0, 8K context cap, best-effort JSON.
- Redefine "airgapped" as local-only inference (no cloud/API/egress) decoupled from packaging; auto-provision (download + verify + hook up) the default models on first use, even airgapped.
- Collapse the two cargo/release channels into one download-on-first-use binary.
- Keep both defaults user-substitutable; preserve single static pure-Rust binary across all four targets.

**Non-Goals:**
- Adopting Ollama or any external runtime/daemon (explicitly rejected).
- Hard strict-JSON / grammar-constrained decoding (candle has no grammar engine; we keep best-effort recovery).
- In-place migration of pre-change (384d) databases.
- GPU/Metal/CUDA acceleration as a shipped default (CPU only, same posture as today).
- Embedding model weights into the binary.

## Decisions

### D1: Extraction runtime = candle in-tree `quantized_qwen3` (bump candle 0.8.4 → ≥0.9.2, prefer 0.10.x)
Quaid already depends on candle for embeddings, so this is a **version bump, not a new ML stack**. candle-transformers `≥0.9.2` ships `models::quantized_qwen3` with the Qwen3-required QK-norm, GGUF/`QMatMul`/`QTensor` loading, and pure-Rust SIMD `Q4_K`/`Q6_K` kernels (AVX2/NEON + scalar). `candle-core` implements the `Q4_K`/`Q6_K` block types that `q4_K_M` uses, and its GGUF reader dispatches per-tensor dtypes. CPU path needs no C/C++ toolchain, preserving the musl/darwin cross-compile story.

- **Alternatives considered.** *llama-cpp-2 (utilityai)* — the only option with a real GBNF/JSON-schema grammar sampler (guaranteed JSON) and native q4_K_M, best-maintained binding; **rejected as default** because it is a second native ML stack requiring cmake+clang on every CI runner and its `build.rs` has no musl branch (hardcodes `libstdc++`), so static aarch64-musl/x86_64-musl is bespoke, painful work — verified against its `build.rs` and cross-rs musl-C++ issues. Kept as the documented fallback *iff* hard strict-JSON ever becomes a requirement. *mistral.rs* — pure-Rust, llguidance constrained decoding, but pins candle 0.10 as a **git rev** that conflicts with Quaid's crates.io candle, and its release matrix is glibc+arm-darwin only (3 of our 4 targets unproven) — strictly more disruptive. *burn/ratchet/kalosm/luminal/crabml/Crane* — each fails a hard axis (no GGUF, no Qwen3, no license, or abandoned).

### D2: New `SlmClient` impl, not a mutation of `SlmRunner`
`SlmRunner` is monomorphic on candle `Phi3Model` (safetensors). Add a new impl (e.g. `slm_gguf.rs`) that loads via `candle_core::quantized::gguf_file::Content::read` + `quantized_qwen3::ModelWeights`, behind the existing open `SlmClient` trait. Consumers (`extractor`, `correction`, `server`, `http`, `vault_sync`) depend only on the trait and need no changes; selection happens at the wiring points (`server.rs` `new_with_slm`, `vault_sync/embedding.rs`). The greedy `Sampling::ArgMax` loop, per-call KV-cache clear, `catch_unwind` panic isolation, and EOS-set handling carry over. EOS comes from GGUF metadata (`tokenizer.ggml.eos_token_id` + `<|im_end|>`) instead of `config.json`. The phi3-only `model_type` gate and the runner-supported-alias guards in `model_lifecycle.rs` are repointed (a GGUF has no HF `config.json`; arch lives in `general.architecture` metadata).

- **Alternative:** branch inside `SlmRunner` — rejected; it would couple two unrelated loaders and weaken the clean trait seam.

### D3: Embedder = candle Qwen3 with last-token pooling + instruction query, new path beside BGE
`Qwen3-Embedding-0.6B` is a Qwen3 decoder, not a BERT encoder: pooling is **last non-pad token**, queries use `Instruct: {task}\nQuery: {query}` (no BGE prefix), output is L2-normalized 1024d. `inference.rs` gains a Qwen3 backend variant and a pooling/query branch selected by model identity; the existing BGE `CandleBert`/`CandleXlmRoberta` paths stay for the retained aliases. Reuses the existing `page_embeddings_vec_1024` table. `EMBEDDER_VERSION` is bumped so `quaid embed --all/--stale` re-embeds exactly once.

- **Alternative:** keep BGE-small default — rejected (the benchmark gap is the entire motivation).

### D4: "Airgapped" = property, not packaging; one channel, provision on first use
Decouple the privacy promise (local-only inference, no egress) from the embedded-bytes mechanism. Delete the `embedded-model`/`online-model` split; ship one binary per platform that resolves models from the local cache and auto-provisions (download + integrity-verify + hook up) the configured default on first use — with progress UI — even under the airgapped posture. Reuse `model_lifecycle.rs`'s content-agnostic manifest/verify/download plumbing; it must additionally accept a single `.gguf` (drop the safetensors-set requirement for that path). Auto-provisioning is **user-triggered / first-use**, never a hidden side effect mid-request — the existing "daemon never silently downloads" requirement in `slm-runtime` is preserved.

- **Alternative:** keep a tiny embedded BGE-small fallback for true zero-network cold start — rejected for now (adds back the dual-channel complexity we're deleting; pre-release lets us go all-in on download-on-first-use). Revisitable if cold-start-offline UX demands it.

### D5: Keep best-effort JSON (status quo), set context explicitly
candle has no constrained-decoding engine; strict JSON stays greedy + `serde_json` + the existing recovery path — the same contract Phi-3 ships today, for a fixed small extraction schema at temp 0. `n_ctx` is set explicitly to 8K to bound the KV cache (the model's 262K trained context would otherwise attempt a multi-GB allocation — a config-OOM that affects any runner).

## Risks / Trade-offs

- **Exact GGUF not yet load-tested** (arch fit confirmed, weights not) → Mitigation: a smoke-test task end-to-end on the real `Qwen3-4B-Instruct-2507-Q4_K_M.gguf` before relying on it; treat as a gate.
- **candle decode throughput unknown** — the survey's "fused/flash-attn CPU kernel" claim was **refuted** by verification; `quantized_qwen3` uses generic candle ops → Mitigation: benchmark tokens/sec on target CPUs; do not assume latency. A 4B model on CPU may be materially slower than Phi-3.5 per token (offset by background/daemon execution and the ~8 GB→~2.5 GB memory win).
- **candle semver churn** — a `DType` enum variant was added in a *patch* release and broke downstream exhaustive matches → Mitigation: add wildcard arms to exhaustive `DType` matches; pin candle deliberately; budget for bump churn. Embedding APIs (`BertModel`/`XLMRobertaModel`) verified byte-stable 0.8.4→0.10.2.
- **KV-cache OOM if `n_ctx` defaults** → Mitigation: D5 — always set the cap explicitly.
- **Strict-JSON is best-effort only** — if extraction reliability turns out to need guaranteed valid JSON, the recommendation flips to the llama-cpp-2 fallback and its full C++/musl cost → Mitigation: monitor JSON-parse failure rate; the `SlmClient` seam makes swapping the runner localized.
- **First-use download UX under airgapped** — a one-time ~2.5 GB (extractor) + ~1.2 GB (embedder) fetch must be discoverable and resumable; offline-without-cache must fail actionably, not degrade silently → Mitigation: progress UI, actionable offline errors, reuse verified cache.
- **Cross-compile regressions from the candle bump** → Mitigation: exercise all four targets in the release gate (single channel now) before tagging.

## Migration Plan

1. Bump `candle-*` and add `DType` wildcard arms; confirm embedding path still compiles/tests on all four targets.
2. Add the Qwen3 embedder backend + pooling/query path; bump `EMBEDDER_VERSION`; default config → `Qwen/Qwen3-Embedding-0.6B` / 1024d.
3. Add the GGUF `SlmClient` impl; teach `model_lifecycle.rs` the single-`.gguf` fetch/verify; repoint the phi3-only gate; default extraction model → `Qwen3-4B-Instruct-2507` q4_K_M.
4. Collapse the cargo channels + release/install surface to one binary; implement auto-provision-on-first-use.
5. Update README/docs/spec/CLAUDE.md.
6. **Rollback**: pre-release, so rollback = revert the branch. For users: re-install the prior binary and re-init (no forward migration exists by design).
7. **Data**: no in-place migration. Users re-init and re-embed (384d→1024d is incompatible).

## Open Questions

- Exact candle version to pin (0.9.2 vs 0.10.x) — resolve by smoke-testing the GGUF + benchmarking decode on each candidate.
- Curated alias name for the default extractor (e.g. `qwen3-4b-2507`) and its pinned GGUF digest/source repo (e.g. an `unsloth/Qwen3-4B-Instruct-2507-GGUF`).
- Whether to retain `QUAID_CHANNEL` as a deprecated no-op or remove it outright from `install.sh`.
- Whether the embedder should keep an offline cold-start fallback (D4 alternative) — defer unless UX testing demands it.
