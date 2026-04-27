# Configurable Embedding Model — Implementation Checklist

**Scope:** Add `QUAID_MODEL` env var and `--model` CLI flag to quaid. Online-model channel only. Airgapped channel unchanged. Closes #44.

---

## Phase A — OpenSpec

- [x] A.1 Create `openspec/changes/configurable-embedding-model/` with `proposal.md`, `tasks.md`, and specs: `model-selection/spec.md`, `memory-config-schema/spec.md`, `model-mismatch-detection/spec.md`.

---

## Phase B — Schema

- [x] B.1 Add `quaid_config` table DDL to `src/schema.sql` (key TEXT PK, value TEXT, STRICT).
- [x] B.2 Update `src/core/db.rs` to create `quaid_config` at init and write `model_id`, `model_alias`, `embedding_dim`, `schema_version` on first init. Make idempotent (no-op if already populated).

---

## Phase C — Model Resolution

- [x] C.1 Add `ModelConfig` struct to `src/core/inference.rs` with fields: `alias`, `model_id`, `embedding_dim`, `sha256_hashes` (Option).
- [x] C.2 Implement `resolve_model(input: &str) -> ModelConfig` resolving aliases (small/base/large/m3) and pass-through HuggingFace IDs.
- [x] C.3 Pin SHA-256 hashes for all four standard aliases. Custom IDs: warn + skip verification.
- [x] C.4 Make `EMBEDDING_DIMENSIONS` runtime (from resolved `ModelConfig`) rather than compile-time const.
- [x] C.5 Update online-model download path to use `ModelConfig.model_id` and `ModelConfig.sha256_hashes`.

---

## Phase D — CLI

- [x] D.1 Add `--model` global flag to clap root command in `src/main.rs`. Reads `QUAID_MODEL` env as default via clap's `env()`.
- [x] D.2 Pass resolved model selection through to DB open and embedding operations.

---

## Phase E — Mismatch Detection

- [x] E.1 On DB open (after `quaid_config` is readable), compare requested model ID vs stored `model_id`. Error with formatted message on mismatch (see spec).
- [x] E.2 Handle missing `quaid_config` gracefully (pre-v0.9.2 DBs): warn + treat as `small`.
- [x] E.3 Airgapped build: if non-small model requested, warn only, continue.

---

## Phase F — Tests (~90% coverage on new code)

- [x] F.1 Unit: `resolve_model` alias resolution for all four aliases + full HuggingFace ID passthrough.
- [x] F.2 Unit: `quaid_config` write/read roundtrip.
- [x] F.3 Unit: mismatch detection returns correct error type.
- [x] F.4 Unit: missing `quaid_config` table returns deprecation warning, not error.
- [x] F.5 Integration: init with `small`, open with `large` → mismatch error.
- [x] F.6 Integration: init with `large`, open with `large` → success.
- [x] F.7 Mock HuggingFace download in all tests (no real network calls).

---

## Phase G — Docs

- [x] G.1 Update `README.md`: add `QUAID_MODEL` to env vars section, add BGE model comparison table (alias, dims, approx size, use case).
- [x] G.2 Update `CLAUDE.md` Embedding model section to describe runtime model selection.
- [x] G.3 Update any `skills/` SKILL.md files that reference the embedding model.
- [x] G.4 Update website docs if applicable. No separate website docs exist in this repo.

---

## Phase H — PR

- [x] H.1 `cargo test` passes with no failures.
- [ ] H.2 Push `feat/44-configurable-model` and open PR against `main`. Title: `feat: configurable embedding model selection via QUAID_MODEL env / --model flag (#44)`. Body references this openspec change directory.

---

## Phase I — PR #47 Review Fixes

- [x] I.1 Make embedding model activation atomic (no zero-active window).
- [x] I.2 Use unique temp download files + safe publish in shared cache.
- [x] I.3 Force hash shim for online-channel CI to keep required tests hermetic.
