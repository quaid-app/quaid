# Fry — SLM model lifecycle batch decision

- Date: 2026-05-05
- Change: `slm-extraction-and-correction`

## Decision

Land the model-cache plumbing around a manifest-verified install path:

1. Resolve friendly aliases (`phi-3.5-mini`, `gemma-3-1b`, `gemma-3-4b`) to pinned Hugging Face repos/revisions.
2. Download required model artifacts into a temporary cache directory first.
3. Verify SHA-256 from source headers when Hugging Face exposes one (notably safetensor blobs), and persist a local `manifest.json` with computed hashes for every downloaded file.
4. Promote the cache with a final rename only after the manifest verifies cleanly, and delete failed temp installs.

## Why

This keeps the landed slice truthful without pretending every upstream metadata file comes with a server-side SHA-256. Large weight blobs still get source-backed hash verification, while the local manifest gives Quaid a deterministic cache-integrity check for later opens and re-pulls. The temp-dir + rename install path also closes the partial-cache seam needed by `quaid extraction enable` and `quaid model pull`.
