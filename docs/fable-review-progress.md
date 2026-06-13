
## MERGED TO MAIN (2026-06-13)
The operator removed branch protection and authorized merging the full stack. All 25 PRs are now
integrated into `main` (92b9018 → ab5cd9b, 85 commits): 19 auto-detected as MERGED, 6 stacked PRs
(#228/#233/#234/#236/#237/#238) CLOSED-as-merged-by-integration (their commits are ancestors of main;
GitHub can't show the merged badge with no remaining diff).

Integration was a local in-order merge with conflict resolution + a full local gate pass before pushing:
clippy `-D warnings` (both channels), fmt, rustdoc, `cargo test --lib` (936 passed), and ~45 integration
suites. Two genuine cross-PR interaction bugs were found and fixed during integration (neither catchable
by any single PR's CI):
1. **Mismatched pooling** — `embed_batch` (page embedding, #237) mean-pooled while queries (#227) CLS-pooled;
   added `cls_pool_and_normalize_batch`. Restored the buried-fact acceptance score (0.386 → passing).
2. **KNN parity oracle** — #239's brute-force test oracle embedded queries with plain `embed()`; production
   now uses `embed_query()` (#227 prefix). Aligned the oracle.
Plus the expected mechanical fixups: struct-field cascades across sibling test initializers (namespace,
rerank/hops, redact, format_version, SearchResult rerank fields), `run_with_batch` arity, MCP
envelope-parse sites in tests, the `core::pages` add/add concatenation deferral (kept #232's arrangement;
#245 contributed the put BEGIN IMMEDIATE fix), and the skills-validator exit-2 false-positive.

**Known CI fragility (not a code defect):** build.rs downloads BGE-small from huggingface.co on cache-cold
runners; HF rate-limits (429) → Check/Test fail spuriously. Re-run clears it. Worth a follow-up: cache or
commit the embedded model asset so the build path never hits HF per-run.
