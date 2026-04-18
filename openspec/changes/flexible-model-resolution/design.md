## Context

`resolve_model()` in `src/core/inference.rs` maps `--model` inputs to `ModelConfig` structs. Currently each known alias carries a pinned HuggingFace commit SHA (`revision`) and a `sha256_hashes` struct with expected file hashes. These values rotted — HF reorganised the large and m3 repos, causing 404s on download. Additionally, aliases `medium` and `max` appear in docs but are absent from the match table.

The dimension-inference path (`hydrate_model_config`) already exists for custom model IDs and reads `embedding_dim` from `config.json` at download time, so the infrastructure for arbitrary model IDs is in place.

## Goals / Non-Goals

**Goals:**
- Remove all pinned revision SHAs and sha256_hashes from known-alias entries
- Add `medium` → `base` and `max` → `m3` aliases to close the doc/code gap
- Accept any `owner/repo` HF model ID without warnings
- Add `gbrain model list` subcommand: static table of known aliases, their HF IDs, dims, and sizes
- Update `--model` help text to mention `gbrain model list`

**Non-Goals:**
- Live HF API queries (rate limits, auth complexity, network dependency)
- Integrity checking of downloaded model files (removed with the SHA tables)
- Changing the DB `brain_config` schema or `model_id` storage format

## Decisions

### Remove all revision pinning and SHA verification for known aliases

**Decision:** Drop `ModelFileHashes`, the four `*_HASHES` constants, and the `revision` field from `ModelConfig`. Known aliases resolve to `(model_id, embedding_dim, revision: None, sha256_hashes: None)`.

**Alternatives considered:**
- *Keep SHAs, update to current commits* — maintenance treadmill; same problem recurs on next HF repo reorg
- *Pin to `"main"` branch* — avoids 404s but still requires manual updates if the branch is renamed or the model is restructured
- *Warn-only on SHA mismatch* — half-measure; still requires maintaining hash tables

**Rationale:** The meaningful reproducibility guarantee is the `model_id` string stored in `brain_config`, validated on every `brain open`. Commit SHAs offer a second layer whose maintenance cost exceeds its benefit for a personal, single-user tool.

### Accept arbitrary HF model IDs silently (no warning)

**Decision:** The `_` catch-all branch in `resolve_model()` currently prints a warning for "unpinned custom" models. Remove the warning — any `owner/repo` string is first-class.

**Rationale:** Users passing full HF IDs (`sentence-transformers/all-MiniLM-L6-v2`) are doing exactly the right thing. The warning incorrectly signals that this is unsafe.

### Add `medium` and `max` as documented aliases

**Decision:** `medium` → `base` (BAAI/bge-base-en-v1.5, 768d), `max` → `m3` (BAAI/bge-m3, 1024d).

**Rationale:** Closes issue #60. Cheaper than updating all docs. Aliases are obvious and consistent with size-based naming.

### `gbrain model list` as a static informational command

**Decision:** New `src/commands/model.rs` with a `list` subcommand. Prints a fixed table from a `KNOWN_MODELS` const slice. No network required.

**Alternatives considered:**
- *Live HF API query via `--remote` flag* — too much complexity (rate limits, pagination, auth tokens) for marginal benefit
- *Include in `--help` text only* — harder to parse, can't pipe/grep

## Risks / Trade-offs

- [Removed SHA verification] downloaded files are no longer integrity-checked → Mitigation: HF CDN uses HTTPS; risk is low for a personal tool; can be re-added opt-in later
- [Alias additions `medium`/`max`] users who already type `--model base` or `--model m3` are unaffected; new aliases are purely additive

## Migration Plan

No DB migration required. `brain_config` stores `model_id` (a string like `BAAI/bge-base-en-v1.5`), not the alias or revision. Existing brains open normally.

Code-only change: update `inference.rs`, add `commands/model.rs`, wire into `main.rs` / `commands/mod.rs`.

## Open Questions

- Should `gbrain model list` output plain text (default) and optionally JSON (`--json`)? Recommend yes for scripting consistency with other commands.
