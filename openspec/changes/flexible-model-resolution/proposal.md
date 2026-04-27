## Why

Hardcoded HuggingFace commit SHAs in `resolve_model()` rot as HF reorganizes repos, causing download failures for `large` and `m3` models. At the same time, undocumented aliases (`medium`, `max`) referenced in docs don't exist in code. The fix is to remove pinned revisions and SHA tables entirely, accept arbitrary HF model IDs, and provide a `--model list` command so users can discover supported aliases without reading source.

## What Changes

- Remove all hardcoded `revision` commit SHAs and `sha256_hashes` from `ModelConfig` and the four `*_HASHES` constants
- Simplify `resolve_model()`: known short aliases expand to HF IDs with no pinned revision; any `owner/repo` string is accepted as a custom model ID
- Add `medium` and `max` as aliases (`medium` → `base`, `max` → `m3`) to match documented behaviour, or remove those mentions from docs
- Add `quaid --model list` (or `quaid model list`) subcommand that prints a static table of known aliases, HF IDs, dimensions, and approximate sizes
- Update CLI `--model` help text to reference `--model list`
- **BREAKING**: `ModelConfig.revision` and `ModelConfig.sha256_hashes` fields removed (internal, no public API impact)

## Capabilities

### New Capabilities

- `model-resolution`: How `--model` inputs are resolved to HF model IDs, including alias expansion, custom ID pass-through, and the `--model list` informational command

### Modified Capabilities

- (none — this is a new capability spec; existing behaviour was undocumented)

## Impact

- `src/core/inference.rs` — `resolve_model()`, `ModelConfig`, hash constants
- `src/commands/` — any command accepting `--model` flag; add `list` subcommand
- `CLAUDE.md`, `AGENTS.md`, docs — alias table references need updating
- No DB schema changes; `model_id` string stored in `quaid_config` is unaffected
