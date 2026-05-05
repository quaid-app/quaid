# Leela — lifecycle truth revision

- **Timestamp:** 2026-05-05T06:49:17.593+08:00
- **Context:** `slm-extraction-and-correction` lifecycle artifact rerevision after Professor's proof-gap review
- **Decision:** `openspec\changes\slm-extraction-and-correction\tasks.md` item `3.2` must describe curated aliases as a shipped per-file mixed-digest pin table, not as a weight-vs-metadata split. The honest contract is that each pinned artifact is verified by either SHA-256 or git-blob-SHA1 according to the alias table, while raw repo-id downloads remain on the weaker server-supplied ETag SHA-256 path where available.
- **Why:** The landed Gemma alias tables pin `tokenizer.json` and `tokenizer.model` by SHA-256, so wording that says tokenizer files are uniformly git-blob-SHA1-verified is false even though the implementation and tests are otherwise correct. This revision stays deliberately narrow because the remaining proposal/design text already describes source-pinned curated aliases at a truthful surface.
