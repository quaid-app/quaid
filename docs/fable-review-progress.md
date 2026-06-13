
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

## Integration verified green (2026-06-13)
Full local validation of merged `main`: clippy `-D warnings` (both channels), fmt, rustdoc,
`cargo test --lib` (936), and the complete default-channel integration suite (**151 suites, 1890 tests,
0 failures**). CI-surfaced failures during integration were ALL stale test expectations from clean
merges (zero product regressions); fixes, by class:
- **Real cross-PR bugs (2):** batch CLS-pooling mismatch (#227×#237); KNN parity oracle missing the
  query-instruction prefix (#239×#227).
- **MCP envelope-parse test sites (#228/#239 added the `{results,…}` envelope):** gap_loop,
  search_hardening, mcp_hops_passthrough, search_confidence, mcp_server inline tests, call.rs inline tests.
- **Struct-field cascades** across sibling test initializers (namespace #232, rerank/hops #228,
  redact #243, format_version #226, SearchResult rerank fields #228) + `run_with_batch` json arg (#225).
- **Behavioral/expectation drift:** put `--json` stdout→stderr (#225×#239); cross-process OCC accepts the
  canonical `ConflictError` prefix (#225); `round_trip_breaks…` inverted now #226 escaping landed (#226×#238);
  skills-validator exit-2 vs clap-error (#240×#241); relevance-floor recalibrated for asymmetric retrieval
  (#224×#227); check/gaps page lookups migrated to `core::pages::resolve` + audit allowlist synced (#228/#229×#232/#231).
- **CI infra:** build.rs now retries HF downloads on 429/5xx/network (cold-runner resilience).
