## 1. Clean up ModelConfig and hash constants

- [x] 1.1 Remove `ModelFileHashes` struct and all four `*_HASHES` constants (`SMALL_HASHES`, `BASE_HASHES`, `LARGE_HASHES`, `M3_HASHES`) from `src/core/inference.rs`
- [x] 1.2 Remove `revision: Option<&'static str>` and `sha256_hashes: Option<ModelFileHashes>` fields from `ModelConfig`
- [x] 1.3 Remove any SHA verification logic in the download path that references `sha256_hashes`

## 2. Simplify resolve_model()

- [x] 2.1 Rewrite `resolve_model()` match arms: known aliases return `ModelConfig` with no revision/hashes
- [x] 2.2 Add `"medium"` arm → `base` (BAAI/bge-base-en-v1.5, 768d)
- [x] 2.3 Add `"max"` arm → `m3` (BAAI/bge-m3, 1024d)
- [x] 2.4 Remove the warning `eprintln!` from the `_` catch-all arm; arbitrary HF IDs are accepted silently
- [x] 2.5 Ensure full HF IDs (e.g. `BAAI/bge-base-en-v1.5`) still normalise to their alias equivalents

## 3. Add `gbrain model list` command

- [x] 3.1 Create `src/commands/model.rs` with a `KNOWN_MODELS` const slice (alias, model_id, dim, size_mb, notes)
- [x] 3.2 Implement plain-text table output for `gbrain model list`
- [x] 3.3 Implement `--json` flag outputting a JSON array
- [x] 3.4 Wire `model` subcommand into `src/commands/mod.rs` and `src/main.rs`

## 4. Update help text and docs

- [x] 4.1 Update `--model` flag description in all commands to mention `gbrain model list`
- [x] 4.2 Update `CLAUDE.md` alias table: add `medium`, `max`; remove SHA/revision references
- [x] 4.3 Update `AGENTS.md` if it references model aliases or revision pinning

## 5. Tests

- [x] 5.1 Add unit tests in `src/core/inference.rs` for `medium` and `max` aliases
- [x] 5.2 Add unit test that full HF IDs normalise to alias equivalents
- [x] 5.3 Add unit test that arbitrary `owner/repo` strings are accepted without error
- [x] 5.4 Verify `cargo test` passes
