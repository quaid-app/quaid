## 1. Dependency bump (candle 0.8.4 → ≥0.9.2)

- [ ] 1.1 Bump `candle-core`, `candle-nn`, `candle-transformers` in `Cargo.toml` to the chosen version (prefer 0.10.x); update `Cargo.lock`
- [ ] 1.2 Add wildcard arms to any exhaustive `candle_core::DType` matches across `src/` to absorb new enum variants
- [ ] 1.3 Confirm the embedding path compiles unchanged: `BertModel::load` / `forward` and `XLMRobertaModel` APIs still resolve
- [ ] 1.4 Build all four release targets (darwin-arm64/x86_64, linux-x86_64/aarch64-musl) to prove the bump keeps the pure-Rust CPU cross-compile green
- [ ] 1.5 Run the existing test suite; fix any breakage from the bump

## 2. Embedding model → Qwen3-Embedding-0.6B

- [ ] 2.1 Add a Qwen3 embedder backend variant in `inference.rs` (candle Qwen3 model, CPU)
- [ ] 2.2 Implement last-token (last non-pad) pooling + L2 normalization for the Qwen3 path, beside the existing BGE CLS pooling
- [ ] 2.3 Implement the instruction-aware query format (`Instruct: {task}\nQuery: {query}`) for Qwen3 queries; leave passages/symmetric comparisons un-prefixed; keep the BGE prefix path intact
- [ ] 2.4 Add `qwen3-0.6b` alias resolution → `Qwen/Qwen3-Embedding-0.6B`, 1024d; make it the default in `resolve_model` / `default_model`; retain BGE aliases as opt-in
- [ ] 2.5 Bump `EMBEDDER_VERSION` so `quaid embed --all/--stale` re-embeds once; confirm the `page_embeddings_vec_1024` table is reused
- [ ] 2.6 Verify embeddings are 1024d, normalized, and that query/passage asymmetry is wired correctly (sanity similarity check on a known pair)

## 3. Extraction runtime → in-process Qwen3-4B GGUF

- [ ] 3.1 Add `slm_gguf.rs`: a new `SlmClient` impl loading via `candle_core::quantized::gguf_file::Content::read` + `quantized_qwen3::ModelWeights`
- [ ] 3.2 Carry over greedy `Sampling::ArgMax` decode, per-call KV-cache clear, and `catch_unwind` → `SlmError::Panic` isolation
- [ ] 3.3 Derive EOS stop tokens from GGUF metadata (`tokenizer.ggml.eos_token_id` + `<|im_end|>`); load the GGUF-embedded tokenizer
- [ ] 3.4 Set `n_ctx` explicitly to 8K to bound the KV cache; assert prompts+generation respect the cap
- [ ] 3.5 Reuse the existing best-effort JSON path (`parse_response` + recovery) unchanged; do not add grammar/constrained decoding
- [ ] 3.6 Repoint the phi3-only `model_type` gate and the runner-supported-alias guards in `model_lifecycle.rs` (use GGUF `general.architecture`, not HF `config.json`)
- [ ] 3.7 Wire runner selection at `server.rs` (`new_with_slm`) and `vault_sync/embedding.rs` to the GGUF `SlmClient`
- [ ] 3.8 Add the default extraction alias (e.g. `qwen3-4b-2507`) → `Qwen3-4B-Instruct-2507` q4_K_M GGUF; pin its source repo + digest

## 4. Model provisioning + airgapped redefinition

- [ ] 4.1 Teach `model_lifecycle.rs` `select_files_to_download` to accept a single `.gguf` (drop the config/tokenizer/safetensors-set requirement for that path); reuse the content-agnostic manifest write+verify
- [ ] 4.2 Implement auto-provision-on-first-use for the embedder (first semantic op) and extractor (`extraction enable` / first job): download + integrity-verify + hook up with progress, even under the airgapped posture
- [ ] 4.3 Preserve "daemon never silently downloads mid-request": provisioning is user-triggered/first-use only; `memory_add_turn` etc. never trigger a fetch
- [ ] 4.4 Ensure offline-without-cache fails with an actionable error naming the model and `quaid model pull <alias>` fallback (no silent hash-shim degradation without surfacing cause)
- [ ] 4.5 Confirm `QUAID_MODEL` / `--model` (embedding) and the extraction alias override still resolve, provision, and load substituted models

## 5. Collapse release channels to one binary

- [ ] 5.1 Remove the `embedded-model` / `online-model` cargo feature split; default to a single download-on-first-use build; drop `include_bytes!` model embedding
- [ ] 5.2 Update the release workflow to emit one `quaid-<platform>` asset (+ `.sha256`) per platform, no `-airgapped`/`-online` suffix
- [ ] 5.3 Update `scripts/install.sh` to resolve the single asset; make `QUAID_CHANNEL` a deprecated no-op (or remove)
- [ ] 5.4 Update npm `postinstall.js` to fetch the single asset; keep the tarball binary-free
- [ ] 5.5 Update the release validation gate to exercise the single build path + a model-provisioning smoke check across all four targets

## 6. Config defaults + schema

- [ ] 6.1 Update `quaid init` defaults in `db.rs` / `schema.sql`: `model_id=Qwen/Qwen3-Embedding-0.6B`, `model_alias=qwen3-0.6b`, `embedding_dim=1024`; bump `schema_version`
- [ ] 6.2 Confirm model-mismatch detection surfaces the 384→1024 incompatibility on pre-change DBs (re-init expected; no in-place migration)

## 7. Smoke test + benchmark (gates)

- [ ] 7.1 End-to-end smoke test: load the real `Qwen3-4B-Instruct-2507-Q4_K_M.gguf` and run a fact-extraction prompt → parseable JSON (arch fit confirmed, weights not yet load-tested — this is a gate)
- [ ] 7.2 Benchmark decode throughput (tokens/sec) on a representative CPU per target; record the latency budget (no fused/flash kernel — do not assume)
- [ ] 7.3 Verify daemon resident memory drops to ~2.5–3 GB at q4_K_M (vs ~8 GB F16 Phi-3.5)
- [ ] 7.4 Run/extend the airgap test to prove zero network calls after the one-time provision

## 8. Documentation

- [ ] 8.1 Update README "Airgapped vs online" + size tables to the single-binary, provision-on-first-use, local-only-inference model
- [ ] 8.2 Update `docs/getting-started.md`, `docs/contributing.md`, `docs/spec.md`, `docs/roadmap_*` (channels, embedded-model claims, default models)
- [ ] 8.3 Update `CLAUDE.md`: embedding-model section, channel description, default models, and the airgapped definition
- [ ] 8.4 Update `--model` / extraction-model help text and any alias tables to reflect Qwen3 defaults
