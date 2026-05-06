# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## 2026-05-06T00:05:41Z — Post-Archive Roadmap Sync

- Updated `website/src/content/docs/contributing/roadmap.mdx` post-team-archive cycle
- Arc 1 marked shipped
- Phase 4 marked complete with deferred vault-sync items moved into deferred language
- Arc 2 marked in progress with namespace isolation, conversation/SLM extraction, and contradiction resolution marked complete
- Remaining build order table pruned to unfinished work only

## Learnings

- The project wants a world-class open-source docs website, not just acceptable docs.
- Docs-site changes also follow the OpenSpec-first workflow when meaningful.
- Examples, IA, and search UX matter as much as prose quality.
- The `docs.yml` workflow previously only deployed on push to `main` — PRs had no build validation. Adding a `pull_request` trigger with a conditional `upload-pages-artifact` step (skipped on PRs) and a `deploy` job gated on `push`/`workflow_dispatch` is the correct GitHub Pages pattern.
- Starlight/Astro handles the repository-relative base path automatically when `GITHUB_ACTIONS=true` and `GITHUB_REPOSITORY` are set — all asset/link URLs pick up `/gigabrain/` without per-page changes.
- An "Install & Status" page is the clearest single anchor for surfacing supported-now vs planned-later distribution channels. It belongs first in the Getting Started nav group.
- Roadmap and README must agree on Phase status. When they diverge, README wins as the more actively-maintained source of truth.
- **Dual-release channel naming:** `airgapped` and `online` are the canonical channel names. "slim" may appear as an informal size descriptor but must not be used as a channel name/label anywhere in public docs.
- **Source-build default is `online`**: `Cargo.toml` sets `default = ["bundled", "online-model"]`. All docs showing `cargo build --release` should label it as the online (default) channel. Airgapped requires explicit `--no-default-features --features bundled,embedded-model`. Installer defaults (shell → airgapped, npm → online) are separate from the source-build default and are correct as documented.
- **Source-build default is `airgapped` (NOT online):** `Cargo.toml` as of `v0.9.1` sets `default = ["bundled", "embedded-model"]`. `cargo build --release` produces the **airgapped** binary. The online build requires `--no-default-features --features bundled,online-model`. History entry from 2026-04-18 was wrong on this point; Bender's rejection corrected it.
- When a Bender rejection disputes the history.md record itself, always re-read Cargo.toml (or the authoritative implementation file) before revising docs. History entries can be stale.
- **Fake terminal output is a high-priority stale signal**: `gbrain serve` showing `"Server active at http://localhost:8080"` was the most misleading single line on the site. Any code block with simulated output should be reviewed on every branch that changes serve behavior.
- **Tool count drift**: When new MCP tools land, update tool count references in: `mcp-server.md` table, `phase3-capabilities.md` description + Related section, `getting-started.mdx` action banner. There are at least four places.
- **Schema version references**: `getting-started.mdx` step 01 prose, `contributing.md` repo layout annotation. These drift every schema bump.
- **vault-sync-engine docs strategy**: Document capabilities as "In progress" in roadmap only until the branch merges. Deferred work (restore, IPC) must be explicitly named as deferred — silence reads as "not planned", explicit listing reads as "coming."
- **Roadmap version targets**: Internal issue numbers (#55 etc.) should not appear in public-facing version target rows.
- **Homepage and user funnel accuracy (2026-04-25):** User-facing docs misled the audience for months about the MCP surface: the snippet claimed 'Server active at http://localhost:8080' but gbrain serve is stdio JSON-RPC, not HTTP. The fix was not a footnote or a clarification — it was a replacement sequence showing the real transport (init → import → serve with no HTTP output). Never ship example code that contradicts the actual behavior; users will spend days debugging the wrong thing. When correcting this, also surface any major workflow steps that were missing from the funnel (e.g., import was omitted from the homepage but is mandatory to see results).
- **MCP tool response schema drift (2026-04-25):** The `brain_collections` example in `mcp-server.md` used fabricated field names (`id`, `path`, `write_target`, `blocker`, `restore_pending`, `ignore_patterns_count`) that don't match `BrainCollectionView`. Always verify docs JSON examples against the actual serialized struct before shipping. The authoritative source is the struct definition in `src/core/vault_sync.rs`.
- **Platform-gated feature docs (2026-04-25):** When a feature is `#[cfg(unix)]` or behind `ensure_unix_platform`, every docs surface advertising that feature must carry an explicit Unix/macOS/Linux-only note — homepage cards, quickstart snippets, and guide prose included. One missing note causes support tickets from Windows users.
- **Quarantine restore partial-land pattern (2026-04-25):** When a feature is partially landed (Unix-only narrow seam), the roadmap should show it in the Landed list with its caveat, not in "Explicitly deferred." Only move something to deferred if zero code is shipped. Partial-land → landed-with-caveats.
- **Roadmap deferred items on archive:** When a change is archived with known deferred scope, always move those items to the Deferred table with one-sentence reasoning. Silence reads as "not planned"; explicit deferred reads as "coming, but not now."
- **Arc badge lag:** Arc-level status badges lag behind phase cards. Check arc badges whenever all phases in an arc complete — "Mostly shipped" needs to become "Shipped" or the top-level signal misleads contributors.
- **Build order pruning on completion:** The build order summary table should only list incomplete items. Completed items create noise for contributors picking up work; the phase cards above the table are the right place for completed status.
- **Benchmark honesty after implementation:** When an implementation ships but the benchmarks haven't been re-run, preserve the pre-implementation baselines as historical context and explicitly note the re-run is pending. Do not claim targets are met; do not remove the targets either.
