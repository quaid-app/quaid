# Decision: Mutually exclusive features require per-channel CI

**Date:** 2026-04-17
**Author:** Fry
**Context:** PR #33 CI failure on `release/v0.9.1-dual-release`

## Problem

`cargo clippy --all-features` and `cargo llvm-cov --all-features` enable both `embedded-model` and `online-model` simultaneously, which hits the `compile_error!()` guard in `src/core/inference.rs`. This is by design — the features are mutually exclusive compile-time channels.

## Decision

1. **Clippy:** Run two separate passes — one with default features (airgapped), one with `--no-default-features --features bundled,online-model` (online). This validates both channels independently.
2. **Coverage:** Run with default features only. Full coverage of both channels requires two separate coverage runs; deferred unless needed.
3. **BERT truncation:** `embed_candle()` now truncates tokenizer output to 512 tokens (BGE-small-en-v1.5 `max_position_embeddings`). This prevents OOB panics on long BEIR documents without changing embedding quality for short inputs.

## Impact

- CI Check job will pass on both channels.
- BEIR regression job will no longer crash on long documents.
- Coverage job runs default features only (slightly less coverage, but no false failure).

## For Bender

Re-check: (1) both clippy steps pass in CI, (2) BEIR regression job completes without the index-select crash, (3) `install.sh` mktemp behavior on macOS (the `-t` fallback flag).
