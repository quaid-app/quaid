# Mom — lifecycle revision decisions

- **Timestamp:** 2026-05-05T06:49:17.593+08:00
- **Scope:** rejected `3.x` model-lifecycle artifact follow-up

## Decisions

1. **Curated aliases verify against source-pinned digests, not response headers.**
   - For curated aliases, every downloaded file now comes from an in-source pin table.
   - LFS artifacts use pinned SHA-256 values; non-LFS artifacts use pinned Git blob SHA-1 object ids from the source repo tree.
   - Raw repo ids stay supported, but their cache manifests are surfaced as manifest-only rather than source-pinned.

2. **The runtime no-silent-fetch seam is a local-cache loader, not the future SLM runner.**
   - `load_model_from_local_cache()` is the batch’s fail-closed runtime seam: it verifies the cache locally and never calls download code.
   - Until `slm.rs` lands, truthful proof is “runtime loader can fail closed without fetching,” not “full runtime inference path already exists.”

3. **Crash cleanup is closed by stale temp-dir scavenging on later installs.**
   - Atomic rename still prevents partial cache promotion.
   - Follow-up installs now remove stale `.alias-download-*` directories while preserving fresh ones so interrupted downloads do not grow disk forever.
