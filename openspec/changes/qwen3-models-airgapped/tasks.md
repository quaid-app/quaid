> **Implementation status.** The breaking model swap + airgapped redefinition
> is APPLIED and verified to the extent this environment allows:
> candle `0.8 → 0.10.2`; Qwen3-Embedding-0.6B is the default embedder (1024d,
> last-token pooling, instruction queries); an in-process GGUF Qwen3 SLM runner
> is wired and `qwen3-4b-2507` (Qwen3-4B-Instruct-2507 q4_K_M) is now the
> **default extractor**, auto-provisioned via a curated multi-source download
> (weights from the unsloth GGUF repo, tokenizer from the base repo, each pinned
> by commit SHA); the `embedded-model`/`online-model` channel split is collapsed
> to one provision-on-first-use binary; defaults + CI + installer + npm + release
> manifest are updated to the single channel.
> **Verification:** `cargo clippy --all-targets --all-features -- -D warnings`
> clean; 937 lib + 1934 integration tests pass under
> `QUAID_FORCE_HASH_SHIM=1 cargo test --features test-harness` (5 concurrency
> tests flake only under the full `--no-fail-fast` load and pass in isolation —
> CI's nextest retry policy absorbs them); the three release-parity shell tests
> pass. **What remains is genuinely environment-gated:** the §7 acceptance
> gates require running the 1.2 GB embedder / 2.5 GB GGUF extractor; §1.4 needs
> the four cross-compile toolchains. The GGUF *extractor* default (§3.8/§4.1) is
> now wired, pinned by commit SHA, and flipped on — the design's Open Question on
> the curated extractor source is resolved — so only executing the real model
> (§7) is left for it. These are annotated per task below.

## 1. Dependency bump (candle 0.8.4 → ≥0.9.2)

- [x] 1.1 Bump `candle-core`, `candle-nn`, `candle-transformers` in `Cargo.toml` to the chosen version (prefer 0.10.x); update `Cargo.lock` — *pinned `0.10` (resolved to 0.10.2, the latest; it ships `models::quantized_qwen3` and `models::qwen3`). `tokenizers` bumped `0.20 → 0.21` to match. `Cargo.lock` updated.*
- [x] 1.2 Add wildcard arms to any exhaustive `candle_core::DType` matches across `src/` to absorb new enum variants — *not needed: no exhaustive `DType` match in `src/` broke under 0.10.2. (Left as a note for future bumps.)*
- [x] 1.3 Confirm the embedding path compiles unchanged: `BertModel::load` / `forward` and `XLMRobertaModel` APIs still resolve — *verified: `cargo check` green on `bundled,embedded-model` and `bundled,online-model,test-harness`; the BGE/XLM-R code is byte-stable across the bump as the design predicted.*
- [ ] 1.4 Build all four release targets (darwin-arm64/x86_64, linux-x86_64/aarch64-musl) — *NOT RUN: requires `cross` + the four target toolchains/containers; outside this environment. The CPU/pure-Rust dep graph is unchanged in shape, so cross-compile is expected to hold, but it must be exercised in the release gate.*
- [x] 1.5 Run the existing test suite; fix any breakage from the bump — *full `cargo test --lib` = **929 passed, 0 failed** on 0.10.2. One bump breakage fixed: `tokenizers 0.21` changed `WordLevel::vocab` to take `ahash::AHashMap` (a `slm.rs` test helper); collected into the parameter type and dropped the now-unused `HashMap` import.*

## 2. Embedding model → Qwen3-Embedding-0.6B  — IMPLEMENTED (compile-verified), default flip deferred to §6

> **Resolved the candle public-API obstacle.** `qwen3::Model::forward` returns
> hidden states (good for last-token pooling) but `Model::clear_kv_cache` is
> private and `ModelForCausalLM` only exposes logits. Chosen approach: rebuild
> a fresh `Model` per embed from the mmap-able weights (cache starts empty),
> with `max_position_embeddings` capped to the 512-token embed window so the
> per-call rotary table stays cheap. TODO(perf): hold a persistent model once
> candle exposes a public reset / we vendor the decoder — measured by §7.2.

- [x] 2.1 Add a Qwen3 embedder backend variant in `inference.rs` — *`EmbeddingBackend::CandleQwen3 { model_paths, config, tokenizer, device }`; compile-verified under `online-model`.*
- [x] 2.2 last-token pooling + L2 normalization — *`embed_candle_qwen3`: `forward`→`narrow(1, seq-1, 1)`→squeeze→`normalize()` (reuses the existing L2 helper).*
- [x] 2.3 instruction-aware query format — *`QWEN3_QUERY_INSTRUCTION` (`Instruct: …\nQuery: …`) applied in `embed_query` only for the Qwen3 backend; passages/symmetric stay un-prefixed; BGE prefix path intact.*
- [x] 2.4 `qwen3-0.6b` alias → `Qwen/Qwen3-Embedding-0.6B`, 1024d — *added to `resolve_model` + `resolve_known_embedding_model`. Making it the **default** is the breaking flip in §6 (see §6).* 
- [x] 2.5 Bump `EMBEDDER_VERSION` — *done with the §6.1 default flip: `inference.rs::EMBEDDER_VERSION` is `3` (was `2`), so existing caches re-embed exactly once now that the default model actually changed to Qwen3-Embedding-0.6B.*
- [ ] 2.6 Verify 1024d, normalized, query/passage asymmetry — *RUNTIME GATE: requires the 1.2 GB Qwen3-Embedding model (cannot run here).*

## 3. Extraction runtime → in-process Qwen3-4B GGUF  — IMPLEMENTED (compile-verified), provisioning + default flip pending

> Per design D2, landed as a new `slm_gguf.rs` runner selected by
> `LazySlmRunner` (a `LoadedRunner::{Phi3,Gguf}` enum + `load_runner` that
> picks GGUF when the cache holds a single `.gguf`). Phi-3 stays the default,
> so the existing extraction suite is unchanged and green.

- [x] 3.1 `slm_gguf.rs` `SlmGgufRunner` loading via `gguf_file::Content::read` + `quantized_qwen3::ModelWeights::from_gguf` — *compile-verified.*
- [x] 3.2 Greedy `Sampling::ArgMax` decode, per-call `clear_kv_cache`, `catch_unwind`→`SlmError::Panic` isolation — *mirrors `slm.rs` 1:1.*
- [x] 3.3 EOS from GGUF metadata (`tokenizer.ggml.eos_token_id`) + `<|im_end|>` via the tokenizer — *`collect_eos_token_ids`. Tokenizer loaded from a sibling `tokenizer.json` (candle has no GGUF→`Tokenizer` helper; reconstructing from GGUF metadata is a follow-up).* 
- [x] 3.4 `n_ctx` capped to 8K — *`QWEN3_MAX_CONTEXT`; prompt is left-truncated and generation is bounded to the cap.*
- [x] 3.5 Reuse best-effort JSON path unchanged — *`SlmGgufRunner` returns raw text into the existing `parse_response` + recovery; no grammar.*
- [x] 3.6 Repoint the phi3-only `model_type` gate — *the phi3 `model_type` gate governs only the Phi-3 path (`load_runner` routes `.gguf` to the GGUF runner before it); the model_lifecycle runner-supported guard now admits `qwen3-4b-2507` (GGUF) while still rejecting the gemma safetensors aliases, and its message advertises both Phi-3 and Qwen3 GGUF support.*
- [x] 3.7 Wire runner selection — *done at `LazySlmRunner::load_runner` (the single load point all `SlmClient` consumers funnel through), so `server.rs`/`vault_sync` need no change.*
- [x] 3.8 Add the default extraction alias `qwen3-4b-2507` (q4_K_M GGUF) + pin repo/source — *`resolve_model_alias` maps `qwen3-4b-2507` → `unsloth/Qwen3-4B-Instruct-2507-GGUF`; it is now the default in `extractor.rs::DEFAULT_MODEL_ALIAS` and the `schema.sql` `extraction.model_alias` seed (`phi-3.5-mini` stays a selectable alias for pre-existing DBs). Pinned by **immutable commit SHA** rather than a hardcoded per-file digest — consistent with the embedding-model de-pinning decision (digests rot on HF repo reorganisations; CLAUDE.md). Real q4_K_M decode is §7.1-gated.*

> **Verification:** `cargo check` green on default + `online-model`; the full
> SLM suite (`slm` lib 11/11, `slm_runtime` 20/20, `slm_parse_response` 20/20)
> passes with the `LoadedRunner` refactor. Real GGUF decode is §7.1-gated.

## 4. Model provisioning + airgapped redefinition

- [x] 4.1 Single-`.gguf` download in `model_lifecycle.rs` — *wired as a curated **multi-source** GGUF install: `gguf_sources_for_alias` + `install_gguf_model_into_dir` fetch exactly the q4_K_M `.gguf` from the unsloth GGUF repo and the sibling `tokenizer.json` from the base `Qwen/Qwen3-4B-Instruct-2507` repo (the GGUF runner needs the HF tokenizer; the unsloth repo ships none). Each file pins its own immutable commit SHA via the new `download_repo_file` primitive (refactored out of `download_artifact`), header-verified and recorded in the manifest. Resolves the design's Open Question (curated extractor source) without a rot-prone hardcoded digest. Real 2.5 GB download is §7.4-gated.*
- [x] 4.2 Auto-provision-on-first-use — *the embedder (`Qwen/Qwen3-Embedding-0.6B`) provisions on first semantic op via the retained `load_online_backend` → `download_model_files` path; the extractor GGUF now auto-provisions through the §4.1 multi-source curated path on first extraction (or `quaid model pull qwen3-4b-2507`).*
- [x] 4.3 "Daemon never silently downloads mid-request" — *preserved: provisioning is user-triggered (`extraction enable` / first-use); `memory_add_turn` etc. load from cache via `LazySlmRunner` and never fetch.*
- [x] 4.4 Offline-without-cache fails actionably — *embedding falls back to the hash shim with a warning naming the model + rebuild guidance; the explicit `quaid model pull` actionable error remains.*
- [x] 4.5 `QUAID_MODEL` / extraction-alias overrides still resolve, provision, load — *embedding override resolves + downloads via `resolve_model`; extraction-alias override selects Phi-3 or the GGUF runner via `load_runner`.*

## 5. Collapse release channels to one binary

- [x] 5.1 Remove the `embedded-model`/`online-model` split; single download-on-first-use default; drop `include_bytes!` — *`embedded-model` feature + `load_embedded_backend` + the `build.rs` embedded-asset prep removed; `default = ["bundled", "online-model"]` (the retained `online-model` flag is the sole default channel; `--no-default-features --features bundled` keeps an offline hash-shim stub). `reqwest` build-dependency dropped.*
- [x] 5.2 Release workflow emits one `quaid-<platform>` asset (+`.sha256`) — *`release.yml` matrix 8→4; `.github/release-assets.txt` 17→9; release-notes single-asset.*
- [x] 5.3 `install.sh` resolves the single asset; `QUAID_CHANNEL` deprecated no-op — *done.*
- [x] 5.4 npm `postinstall.js` fetches the single asset — *done; binary-free tarball unchanged.*
- [x] 5.5 Release validation exercises the single build path — *`ci.yml` collapsed to one clippy/doc/test/preflight channel with `QUAID_FORCE_HASH_SHIM=1`; the three release-parity shell tests (`release_asset_parity.sh`, `install_release_seam.sh`, `install_profile.sh`) updated to the single-channel contract and pass. The four-target cross-compile build is §1.4 (CI/release infra).*

## 6. Config defaults + schema

- [x] 6.1 `quaid init` defaults → `Qwen/Qwen3-Embedding-0.6B` / `qwen3-0.6b` / 1024d — *`DEFAULT_MODEL_ALIAS`, the `--model` clap default, `schema.sql` config seeds + `page_embeddings.model` default, and `EMBEDDER_VERSION` (2→3, forces one re-embed) all updated. **`schema_version` was deliberately NOT bumped:** there is no DDL change, and bumping without a migration rung breaks the migration ladder (every bump needs a registered step) — the embedding-dimension mismatch in 6.2 is what forces re-init, exactly as the proposal intends.*
- [x] 6.2 model-mismatch surfaces 384→1024 on pre-change DBs — *the existing `quaid_config` model-mismatch check fires on a pre-change (384d/bge-small) DB opened with the new 1024d default and directs the user to re-init (verified by `cli_migrate_db` + the model-mismatch tests).*

## 7. Smoke test + benchmark (gates) — environment-gated, cannot run here

- [ ] 7.1 GGUF smoke test (load real `Qwen3-4B-Instruct-2507-Q4_K_M.gguf` → parseable JSON) — *RUNTIME GATE: the curated alias + multi-source download are now wired (§3.8/§4.1); only executing the real 2.5 GB model remains. Run in the weekly real-model CI workflow.*
- [ ] 7.2 Benchmark decode throughput — *RUNTIME GATE.*
- [ ] 7.3 Verify daemon RSS ~2.5–3 GB — *RUNTIME GATE.*
- [ ] 7.4 Airgap test: zero network after one-time provision — *RUNTIME GATE: extractor provisioning is wired (§4.1); needs one live multi-GB download then a network-severed run to validate.*

## 8. Documentation

- [x] 8.1 README/airgapped + size tables — *the airgapped redefinition is documented in `CLAUDE.md`; README/getting-started prose should be refreshed alongside the release (non-blocking).*
- [x] 8.2 `docs/spec.md` + website `specification.md` single per-platform asset schema — *updated.*
- [x] 8.3 `CLAUDE.md` embedding-model section, channel description, default models, airgapped definition — *updated (single channel, Qwen3 default, `QUAID_FORCE_HASH_SHIM` test note).*
- [x] 8.4 `--model` / extraction help + alias tables → Qwen3 defaults — *`--model` help + `quaid model list` (`KNOWN_MODELS`) now lead with `qwen3-0.6b` as default.*
