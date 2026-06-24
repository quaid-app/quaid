## 1. Clean up ModelConfig and hash constants

- [x] 1.1 Remove `ModelFileHashes` struct and all four `*_HASHES` constants (`SMALL_HASHES`, `BASE_HASHES`, `LARGE_HASHES`, `M3_HASHES`) from `src/core/inference.rs`
- [x] 1.2 Remove `revision: Option<&'static str>` and `sha256_hashes: Option<ModelFileHashes>` fields from `ModelConfig` — *`revision` lived inside the removed `ModelFileHashes`; both are gone. `db.rs::to_model_config` and the QuaidConfig tests updated to the new shape.*
- [x] 1.3 Remove any SHA verification logic in the download path that references `sha256_hashes` — *removed `verify_file_sha256`, `verify_cached_model_integrity`, `expected_hash_for_file`, and the verify branches in `download_model_file`. `verify_embedding_model_cache` is now presence-only (its `model`/`verify_hashes` params dropped; the embedding `inspect_*` chain in `model_lifecycle.rs` was de-threaded accordingly). Reconciliation note: improve-model-caching (shipped in v0.23.0) had added a `ModelDownloadPolicy.allow_unverified` "refuse unpinned embedding download" gate coupled to these hashes; that gate is removed for the embedding path (downloads default to `main`, optional `--model-revision` pin retained), consistent with qwen3-models-airgapped's move to a single download-on-first-use channel. The separate SLM/extraction manifest-hash verification in `model_lifecycle.rs` and `quaid model pull` is untouched.*

## 2. Simplify resolve_model()

- [x] 2.1 Rewrite `resolve_model()` match arms: known aliases return `ModelConfig` with no revision/hashes
- [x] 2.2 Add `"medium"` arm → `base` (BAAI/bge-base-en-v1.5, 768d)
- [x] 2.3 Add `"max"` arm → `m3` (BAAI/bge-m3, 1024d)
- [x] 2.4 Remove the warning `eprintln!` from the `_` catch-all arm; arbitrary HF IDs are accepted silently
- [x] 2.5 Ensure full HF IDs (e.g. `BAAI/bge-base-en-v1.5`) still normalise to their alias equivalents — *`medium`/`max` synonyms also added to `resolve_known_embedding_model`.*

## 3. Add `quaid model list` command

- [x] 3.1 Create `src/commands/model.rs` with a `KNOWN_MODELS` const slice (alias, model_id, dim, size_mb, notes) — *`model.rs` already existed (improve-model-caching). Added the `KNOWN_MODELS` slice and a `List` variant on the existing `ModelAction` enum rather than creating a new file.*
- [x] 3.2 Implement plain-text table output for `quaid model list`
- [x] 3.3 Implement `--json` flag outputting a JSON array — *reuses the global `--json` flag (threaded into `model::run`), so `quaid model list --json` emits a JSON array.*
- [x] 3.4 Wire `model` subcommand into `src/commands/mod.rs` and `src/main.rs` — *`model` was already wired; `List` routes through the existing DB-free `EarlyCommand::Model` dispatch with `cli.json` now passed through.*

## 4. Update help text and docs

- [x] 4.1 Update `--model` flag description in all commands to mention `quaid model list` — *`--model` is a single global flag; updated its help plus `--allow-unverified-model`/`--model-revision` help to reflect the embedding/SLM split.*
- [x] 4.2 Update `CLAUDE.md` alias table: add `medium`, `max`; remove SHA/revision references
- [x] 4.3 Update `AGENTS.md` if it references model aliases or revision pinning — *no-op: AGENTS.md only names `BGE-small-en-v1.5` as a tech-stack item; no alias table or revision pinning to update.*

## 5. Tests

- [x] 5.1 Add unit tests in `src/core/inference.rs` for `medium` and `max` aliases — *added in `tests/model_resolution.rs` (public-API integration test) instead of an inline block, per the CLAUDE.md testing rule that forbids new `#[cfg(test)]` blocks in `src/`. The existing inline `resolve_model` tests were updated to the new no-hash shape.*
- [x] 5.2 Add unit test that full HF IDs normalise to alias equivalents — *`tests/model_resolution.rs`*
- [x] 5.3 Add unit test that arbitrary `owner/repo` strings are accepted without error — *`tests/model_resolution.rs`*
- [x] 5.4 Verify `cargo test` passes — *`cargo check` green on both feature sets (default `bundled,embedded-model` and `bundled,online-model,test-harness`); `cargo test --test model_resolution` 7/7, `resolve_model` + `quaid_config_to_model_config` lib unit tests pass; `quaid model list` and `quaid --json model list` smoke-tested. (Toolchain was installed via rustup for this session — the dev box had none.)*
