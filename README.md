# GigaBrain

> Open-source personal knowledge brain. SQLite + FTS5 + vector embeddings in one file. Thin CLI harness, fat skill files. MCP-ready from day one. Runs anywhere. No API keys, no Docker. Airgapped + online release channels with configurable BGE models in the online build.

**Status:** `v0.9.7` (release candidate) — current vault-sync surface plus the macOS release-build fix and a canonical release-asset contract shared across installer, workflow, docs, and release checks. [See the roadmap →](#roadmap)

---

Inspired by [Garry Tan's GBrain work](https://gist.github.com/garrytan/49c88e83cf8d7ae95e087426368809cb), GigaBrain adapts the same core concept — a personal knowledge brain with compiled truth and append-only timelines — to a local-first Rust + SQLite architecture built for portable, offline use. No API keys. No internet required. No Docker. One static binary, drop it anywhere.

## Roadmap

GigaBrain is built in explicit phases. Each phase has a hard gate — no phase begins until the previous one is reviewed and merged.

| Phase | Status | What ships |
| ----- | ------ | ---------- |
| **Sprint 0** — Repository scaffold | ✅ Complete | `Cargo.toml`, module stubs, `schema.sql`, skill stubs, CI/CD workflows |
| **Phase 1** — Core storage + CLI | ✅ Complete | `gbrain init`, `import`, `get`, `put`, `search`, local embeddings, hybrid search, MCP server, `query`, `compact` |
| **Phase 2** — Intelligence layer | ✅ Complete | `link`, `graph`, `check`, `gaps`; temporal links, contradiction detection, progressive retrieval, novelty checking, knowledge gaps |
| **Phase 3** — Skills, Benchmarks + Polish | ✅ Complete (`v0.9.5` — flexible model resolution + configurable online-model selection) | All 8 skills production-ready, 16 MCP tools, BEIR/corpus-reality/concurrency harnesses, `validate`/`call`/`pipe`/`skills doctor` CLI |
| **vault-sync-engine** — Collections, live-sync, write safety | 🚢 Initial ship (`v0.9.6` — Unix/macOS/Linux) | Collections model, stat-diff reconciler, file watcher, quarantine `list`/`export`/`discard|restore`, write-through `brain_put`, `brain_collections` MCP tool; Windows `serve`/restore and IPC deferred |

OpenSpec change proposals for all four phases are in [`openspec/changes/`](openspec/changes/). Review them before contributing — they are the design record for every major decision.

---

## Why

Git doesn't scale past ~5,000 markdown files. At that size, a wiki-brain becomes slow to clone, painful to search, and unusable for structured queries. Full-text search requires grep. Semantic search requires an external vector database. Cross-references are just markdown links with no queryable graph.

Every existing knowledge tool (Obsidian, Notion, RAG frameworks) either requires a GUI, locks your data in a SaaS platform, or needs an internet connection and API keys. GigaBrain is designed for an agent-first world where your knowledge layer needs to:

- Live in a single file you own completely
- Do full-text **and** semantic search natively
- Expose an MCP server for any AI client
- Work on a plane, in an air-gapped environment, with zero ongoing costs

## How it works

Two core ideas, borrowed from Andrej Karpathy's compiled knowledge model:

**Above the line — compiled truth.** Always current. Rewritten when new information arrives. The intelligence assessment: what we know now.

**Below the line — timeline.** Append-only. Never rewritten. The evidence base: what happened and when.

Every knowledge page is a markdown file with this structure. GigaBrain stores them in a single SQLite database with FTS5 full-text search, vector embeddings for semantic search, and a typed link graph — all in one `.db` file.

**Thin harness, fat skills.** The binary is plumbing. The intelligence lives in `SKILL.md` files that any agent reads at session start. Workflows, heuristics, edge cases — all in plain markdown, not compiled code. Swap or extend skills without rebuilding.

## Features

- **SQLite everything** — FTS5 full-text search, `sqlite-vec` vector similarity, typed link graph — all in one `brain.db` file
- **Local embeddings** — BAAI BGE family via [candle](https://github.com/huggingface/candle) (pure Rust, no ONNX). No OpenAI API key; online builds only need internet for the initial model download
- **MCP server** — `gbrain serve` exposes 17 tools over stdio JSON-RPC 2.0 in `v0.9.6` (Unix/macOS/Linux on the vault-sync line). Works with Claude Code and other MCP-compatible agents
- **Hybrid search** — FTS5 keyword + vector semantic search with set-union merge, exact-match short-circuit, and optional palace-style hierarchical filtering
- **Live file watcher** *(v0.9.6, Unix/macOS/Linux)* — `gbrain serve` runs a per-collection watcher with 1.5 s debounce and reconcile-backed flushes so the brain stays current as you edit in Obsidian or any editor
- **Collection management** *(v0.9.6)* — attach one or more vaults with `gbrain collection add`; per-collection writable/read-only, ignore patterns via `.gbrainignore`, and `<collection>::<slug>` routing across all CLI/MCP surfaces
- **Quarantine lifecycle** *(v0.9.6)* — deleted or renamed pages with DB-only state (links, assertions, gaps) are quarantined rather than hard-deleted; inspect, export, discard, or narrowly restore on Unix via `gbrain collection quarantine ...`
- **Progressive retrieval** — token-budget-gated content expansion (summary → section → full page)
- **Temporal knowledge graph** — typed links with validity windows, contradiction detection via assertions
- **Knowledge gap tracking** — agent logs what it can't answer; research skill resolves gaps later
- **Fat skills** — all 8 agent workflows live in markdown SKILL.md files, embedded in the binary and overridable
- **Integrity validation** — `gbrain validate --all` checks links, assertions, and embeddings
- **JSONL streaming** — `gbrain pipe` for shell pipeline automation; `gbrain call` for raw MCP tool invocation
- **Configurable BGE models** — `GBRAIN_MODEL` / `--model` select `small` (default), `base`, `large`, `m3`, or a full Hugging Face model ID in the `online-model` build
- **Dual release channels** — `airgapped` embeds BGE-small for offline use; `online` stays slimmer and downloads/caches the selected model on first semantic use

## Tech stack

| Component | Choice |
| --------- | ------ |
| Language | Rust |
| Database | SQLite via `rusqlite` (bundled) |
| Full-text search | FTS5 (built into SQLite) |
| Vector search | `sqlite-vec` (statically linked) |
| Embeddings | `candle` + BGE-small/base/large/m3 (pure Rust, local) |
| CLI | `clap` |
| MCP server | `rmcp` (stdio transport) |

## Quick start

> `v0.9.7` keeps the current vault-sync surface and hardens release delivery: macOS release preflight now checks both channels and the public asset contract is centralized around `gbrain-<platform>-<channel>`.

### Install options

| Method | Status |
| ------ | ------ |
| Build from source (`cargo build --release`) | ✅ Available now — airgapped default |
| GitHub Release binary (macOS ARM/x86, Linux x86_64/ARM64) | ✅ Available — `v0.9.7` airgapped + online assets |
| `npm install -g gbrain` | 🚧 Staged — online channel by default once published |
| One-command curl installer | ✅ Available — airgapped by default; set `GBRAIN_CHANNEL=online` for the online asset |

**Build from source** defaults to the airgapped channel. **GitHub Releases** and the **shell installer** expose both channels for `v0.9.7` using the canonical `gbrain-<platform>-<channel>` asset names. The npm package remains a single wrapper package and targets the `online` channel by default.

Install with the shell script:

```bash
curl -fsSL https://raw.githubusercontent.com/macro88/gigabrain/main/scripts/install.sh | sh

# Online channel instead of the default airgapped channel
curl -fsSL https://raw.githubusercontent.com/macro88/gigabrain/main/scripts/install.sh \
  | GBRAIN_CHANNEL=online sh
```

> The installer automatically writes `PATH` and `GBRAIN_DB` exports to your shell profile
> (`~/.zshrc`, `~/.bash_profile` on macOS / `~/.bashrc` on Linux, or `~/.profile`) so gbrain
> works immediately in new sessions.
> To skip profile writes (e.g. in CI), pipe with `GBRAIN_NO_PROFILE=1 sh` or pass `--no-profile`
> with the two-step method:
> ```bash
> curl -fsSL https://raw.githubusercontent.com/macro88/gigabrain/main/scripts/install.sh | GBRAIN_NO_PROFILE=1 sh
> # two-step (download first, then run with flag):
> curl -fsSL https://raw.githubusercontent.com/macro88/gigabrain/main/scripts/install.sh \
>   -o gbrain-install.sh && sh gbrain-install.sh --no-profile
> ```

**Sandboxed / agent environments** — if your security sandbox blocks piping remote scripts
directly to `sh`, download first, then run:

```bash
curl -fsSL https://raw.githubusercontent.com/macro88/gigabrain/main/scripts/install.sh \
  -o gbrain-install.sh && sh gbrain-install.sh
```

Download a pre-built binary from GitHub Releases:

```bash
VERSION="v0.9.7"
PLATFORM="darwin-arm64"   # darwin-arm64 | darwin-x86_64 | linux-x86_64 | linux-aarch64
ASSET="gbrain-${PLATFORM}-airgapped"   # or: gbrain-${PLATFORM}-online
curl -fsSL "https://github.com/macro88/gigabrain/releases/download/${VERSION}/${ASSET}" -o "${ASSET}"
curl -fsSL "https://github.com/macro88/gigabrain/releases/download/${VERSION}/${ASSET}.sha256" -o "${ASSET}.sha256"
shasum -a 256 --check "${ASSET}.sha256"
# Option A: install for the current user
mkdir -p "${HOME}/.local/bin"
mv "${ASSET}" "${HOME}/.local/bin/gbrain"
chmod +x "${HOME}/.local/bin/gbrain"

# Option B: install system-wide (requires root)
sudo install -m 755 "${ASSET}" /usr/local/bin/gbrain
```

Or build from source:

```bash
git clone https://github.com/macro88/gigabrain
cd gigabrain
cargo build --release
# Airgapped channel (default) — embeds BGE-small-en-v1.5 for offline use

cargo build --release --no-default-features --features bundled,online-model
# Online channel — downloads/caches the selected model on first semantic use
```

### Embedding model selection

```bash
# Default remains BGE-small-en-v1.5
gbrain query "stablecoin regulation"

# Environment variable
GBRAIN_MODEL=large gbrain query "stablecoin regulation"

# CLI flag overrides the environment variable
GBRAIN_MODEL=base gbrain --model m3 query "stablecoin regulation"
```

`GBRAIN_MODEL` and `--model` are supported in the `online-model` build. In the default airgapped build they are a warning-only no-op and GigaBrain continues with embedded `small`.

| Alias | Model ID | Dimensions | Approx size | Use case |
| ----- | -------- | ---------- | ----------- | -------- |
| `small` | `BAAI/bge-small-en-v1.5` | 384 | 130 MB | Default, fastest, lowest memory |
| `base` | `BAAI/bge-base-en-v1.5` | 768 | 438 MB | Better recall on larger corpora |
| `large` | `BAAI/bge-large-en-v1.5` | 1024 | 1.34 GB | Highest English recall, slower |
| `m3` | `BAAI/bge-m3` | 1024 | 2.27 GB | Multilingual retrieval |

If you initialize a DB with one model and later open it with another, GigaBrain errors before any command proceeds. Switching models requires a new DB initialization.

### Environment variables

| Variable | Purpose |
| -------- | ------- |
| `GBRAIN_DB` | Default database path for all commands |
| `GBRAIN_MODEL` | Embedding model alias or full Hugging Face model ID for the online build |
| `GBRAIN_CHANNEL` | Installer channel selection (`airgapped` or `online`) |
| `GBRAIN_WATCH_DEBOUNCE_MS` | Watcher debounce window in milliseconds (default `1500`) |
| `GBRAIN_QUARANTINE_TTL_DAYS` | Auto-discard TTL for clean quarantined pages (default `30`) |
| `GBRAIN_RAW_IMPORTS_KEEP` | Per-page raw import history cap (default `10`) |
| `GBRAIN_RAW_IMPORTS_TTL_DAYS` | TTL for inactive raw import rows (default `90`) |
| `GBRAIN_RAW_IMPORTS_KEEP_ALL` | Set to `1` to disable raw import GC |
| `GBRAIN_FULL_HASH_AUDIT_DAYS` | Rehash interval for `collection audit` (default `7`) |

---

## Usage

> All Phase 1, 2, and 3 commands are implemented. See [`docs/spec.md`](docs/spec.md) for full command signatures.

```bash
# Create a new brain
gbrain init ~/brain.db

# Import an existing markdown directory
gbrain import /path/to/notes/ --db ~/brain.db

# Full-text search
gbrain search "river ai"

# Semantic/hybrid query
gbrain query "who has worked with Jensen Huang?"

# Read a page
gbrain get people/pedro-franceschi

# Write/update a page
cat updated.md | gbrain put people/pedro-franceschi

# Collection-qualified slug (v0.9.6)
gbrain get "work::people/pedro-franceschi"

# Create a typed, temporal cross-reference
gbrain link people/pedro-franceschi companies/brex \
  --relationship founded \
  --valid-from 2017-01-01

# N-hop graph neighbourhood
gbrain graph people/pedro-franceschi --depth 2

# List unresolved knowledge gaps
gbrain gaps

# Run contradiction checks
gbrain check --all

# Brain stats
gbrain stats

# Start MCP server (stdio; Unix/macOS/Linux in v0.9.6)
gbrain serve

# Compact WAL → single file for backup/transport
gbrain compact

# Validate brain integrity (Phase 3)
gbrain validate --all          # all checks
gbrain validate --links        # referential integrity only
gbrain validate --assertions   # assertion dedup only
gbrain validate --embeddings   # embedding model consistency only

# Raw MCP tool invocation from CLI (Phase 3)
gbrain call brain_stats '{}'

# JSONL streaming mode for shell pipelines (Phase 3)
echo '{"tool":"brain_search","input":{"query":"machine learning"}}' | gbrain pipe

# Skill inspection (Phase 3)
gbrain skills list             # list active skills and resolution order
gbrain skills doctor           # verify skill hashes and shadowing

# --- Collection management (v0.9.6) ---

# Attach a vault directory as a collection
gbrain collection add work /path/to/my-obsidian-vault

# List all collections
gbrain collection list

# Show extended collection status (ignore parse errors, recovery state, quarantine count)
gbrain collection info work

# Run a stat-diff reconcile against the active root
gbrain collection sync work

# Manage ignore patterns (.gbrainignore wrapper)
gbrain collection ignore add work "drafts/**"
gbrain collection ignore list work
gbrain collection ignore remove work "drafts/**"
gbrain collection ignore clear work --confirm

# Quarantine — pages with DB-only state are quarantined rather than deleted
gbrain collection quarantine list work
gbrain collection quarantine export work --out quarantine.json
gbrain collection quarantine discard work <page-slug>

# Restore a vault root from a backup (offline path; leaves collection in pending-attach until finalized)
gbrain collection restore work /path/to/backup-vault
gbrain collection sync work --finalize-pending   # attach and reopen writes
```

## MCP integration

Add to your MCP client config (e.g. Claude Code):

```json
{
  "mcpServers": {
    "gbrain": {
      "command": "gbrain",
      "args": ["serve"],
      "env": { "GBRAIN_DB": "/path/to/brain.db" }
    }
  }
}
```

> **Unix only in `v0.9.6`.** `gbrain serve` returns `UnsupportedPlatformError` on Windows in the vault-sync release line because the command now owns the watcher, lease, and startup-recovery runtime. Portable CLI reads/searches still work on Windows; MCP hosting via `gbrain serve` is deferred until a safe non-Unix runtime contract exists.

**Phase 1 tools (core read/write):** `brain_get`, `brain_put`, `brain_query`, `brain_search`, `brain_list`

**Phase 2 tools (intelligence layer):** `brain_link`, `brain_link_close`, `brain_backlinks`, `brain_graph`, `brain_check`, `brain_timeline`, `brain_tags`

**Phase 3 tools (gaps, stats, raw data):** `brain_gap`, `brain_gaps`, `brain_stats`, `brain_raw`

**vault-sync tools (collections):** `brain_collections` — returns per-collection status, state, ignore diagnostics, and recovery flags

All 17 tools are available in the current `v0.9.6` release when you run `gbrain serve` on Unix/macOS/Linux.

## Skills

Skills are markdown files that tell agents how to use GigaBrain. They live in `skills/` and are embedded in the binary by default, extracted to `~/.gbrain/skills/` on first run. Drop a custom `SKILL.md` in your working directory to override any default.

All 8 skills are production-ready as of Phase 3.

| Skill | Purpose |
| ----- | ------- |
| `skills/ingest/` | Meeting transcripts, articles, documents |
| `skills/query/` | Search and synthesis workflows |
| `skills/maintain/` | Lint, contradiction detection, orphan pages |
| `skills/enrich/` | External API enrichment (Crustdata, Exa, etc.) |
| `skills/briefing/` | Daily briefing compilation |
| `skills/alerts/` | Interrupt-driven notifications |
| `skills/research/` | Knowledge gap resolution |
| `skills/upgrade/` | Agent-guided binary and skill updates |

```bash
# Check active skills and resolution order
gbrain skills list

# Verify skill hashes and shadowing
gbrain skills doctor
```

## Page types

`person`, `company`, `deal`, `project`, `concept`, `original`, `source`, `media`, `decision`, `commitment`, `action_item`, `area`, `resource`, `archive`, `journal`

The `original` type is for your own thinking — distinct from compiled external intelligence.

### PARA folder inference

When you run `gbrain import`, page types are resolved in three tiers:

1. **Frontmatter `type:` field** — if your file includes `type: project` in YAML frontmatter, that wins. Blank or null values fall through to tier 2.
2. **Top-level folder inference** — GigaBrain infers type from the first folder in the path, supporting PARA and common Obsidian vault layouts:

| Folder name | Inferred type |
|-------------|---------------|
| `Projects` (or `1. Projects`) | `project` |
| `Areas` (or `2. Areas`) | `area` |
| `Resources` (or `3. Resources`) | `resource` |
| `Archives` (or `4. Archives`) | `archive` |
| `Journal` / `Journals` | `journal` |
| `People` | `person` |
| `Companies` / `Orgs` | `company` |

Folder matching is case-insensitive and strips leading numeric prefixes (e.g. `1. `, `02. `).

3. **Default** — falls back to `concept` if no folder match and no frontmatter type.

To override inference, add `type: <your_type>` to the file's YAML frontmatter.

## Contributing

GigaBrain is open for contributions. All three core phases have shipped. The current release is `v0.9.6`, which lands the first vault-sync slice: collections, Unix-gated `gbrain serve`, live-sync watcher, quarantine tooling, and narrow Unix restore on top of the prior dual-channel/model-selection work.

**How we work:**

1. **Read the spec first.** [`docs/spec.md`](docs/spec.md) is the authoritative design document — schema, algorithms, CLI surface, skill design, benchmarks. Everything is defined there.
2. **Proposals before code.** Every meaningful change (code, docs, tests, benchmarks) requires an OpenSpec change proposal in [`openspec/changes/`](openspec/changes/) following the instructions in [`openspec/`](openspec/). This is the design record before implementation.
3. **Check the archived proposals.** Phase proposals in [`openspec/changes/archive/`](openspec/changes/archive/) are the full design record for all shipped phases.
4. **CI gates everything.** PRs must pass `cargo check` + `cargo test` before review.

**Good first contributions:**
- Improve skill content in [`skills/`](skills/) — override patterns, edge-case guidance
- Add fixture pages to [`tests/fixtures/`](tests/fixtures/) for expanded benchmark coverage
- Contribute advisory benchmark results (LongMemEval, LoCoMo, Ragas) from your own environment

**Ground rules:**
- Keep PRs scoped to a single concern
- Document decisions — don't leave reasoning in PR comments alone

---

## Build from source

```bash
# Debug
cargo build

# Release (airgapped default — embeds BGE-small-en-v1.5 for offline use)
cargo build --release

# Online release (downloads/caches the selected BGE model on first semantic use)
cargo build --release --no-default-features --features bundled,online-model

# Cross-compile
cargo install cross
cross build --release --target aarch64-apple-darwin           # macOS ARM
cross build --release --target x86_64-unknown-linux-musl      # Linux x86_64 (static)
cross build --release --target aarch64-unknown-linux-musl     # Linux ARM64 (static)

# Tests
cargo test
```

## Non-goals (v1)

- **Not collaborative** — single-user, single-writer. No auth, no RBAC.
- **Not a sync product** — `rsync`/`scp` is the transport. No CRDTs.
- **Not a full graph database** — typed temporal links, not Cypher queries.
- **Not a note-taking app** — structured knowledge pages. Use Obsidian for freeform notes.
- **Not multimodal** — text only.

## Spec and design documents

- [`docs/spec.md`](docs/spec.md) — full technical specification: schema, search algorithms, CLI surface, skill design, benchmarks, and design decisions
- [`openspec/changes/`](openspec/changes/) — structured change proposals for all four phases; the design record before implementation
- [`AGENTS.md`](AGENTS.md) — orientation for AI agents working in this repo
- [`CLAUDE.md`](CLAUDE.md) — Claude-specific context and conventions

## Acknowledgements

This project is explicitly inspired by [Garry Tan's GBrain](https://gist.github.com/garrytan/49c88e83cf8d7ae95e087426368809cb). Garry's TypeScript/Bun implementation and evolving skill packs were the original catalyst. GigaBrain takes the same architecture — SQLite + FTS5 + vector search + MCP + fat markdown skills — and builds it in Rust for a fully local, truly static, zero-dependency binary. Same brain, different stack, different deployment story.

Additional techniques sourced from [MemPalace](https://arxiv.org/abs/2410.10674), [OMNIMEM](https://arxiv.org/abs/2406.16026), and [agentmemory](https://github.com/AgentOps-AI/agentmemory) research.

## License

MIT
