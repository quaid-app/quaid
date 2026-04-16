# GigaBrain

> Open-source personal knowledge brain. SQLite + FTS5 + vector embeddings in one file. Thin CLI harness, fat skill files. MCP-ready from day one. Runs anywhere. No API keys, no internet, no Docker. Truly static single binary.

**Status:** `v0.9.0` test release in progress — Phase 3 is complete, and the simplified-install rollout is landing with the shell installer first. [See the roadmap →](#roadmap)

---

Inspired by [Garry Tan's GBrain work](https://gist.github.com/garrytan/49c88e83cf8d7ae95e087426368809cb), GigaBrain adapts the same core concept — a personal knowledge brain with compiled truth and append-only timelines — to a local-first Rust + SQLite architecture built for portable, offline use. No API keys. No internet required. No Docker. One static binary, drop it anywhere.

## Roadmap

GigaBrain is built in explicit phases. Each phase has a hard gate — no phase begins until the previous one is reviewed and merged.

| Phase | Status | What ships |
| ----- | ------ | ---------- |
| **Sprint 0** — Repository scaffold | ✅ Complete | `Cargo.toml`, module stubs, `schema.sql`, skill stubs, CI/CD workflows |
| **Phase 1** — Core storage + CLI | ✅ Complete | `gbrain init`, `import`, `get`, `put`, `search`, local embeddings, hybrid search, MCP server, `query`, `compact` |
| **Phase 2** — Intelligence layer | ✅ Complete | `link`, `graph`, `check`, `gaps`; temporal links, contradiction detection, progressive retrieval, novelty checking, knowledge gaps |
| **Phase 3** — Skills, Benchmarks + Polish | ✅ Complete (`v0.9.0` test release) | All 8 skills production-ready, 16 MCP tools, BEIR/corpus-reality/concurrency harnesses, `validate`/`call`/`pipe`/`skills doctor` CLI |

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

- **Single static binary** — ~90MB including embedded BGE-small-en-v1.5 model weights. Zero runtime dependencies.
- **SQLite everything** — FTS5 full-text search, `sqlite-vec` vector similarity, typed link graph — all in one `brain.db` file
- **Local embeddings** — BGE-small-en-v1.5 via [candle](https://github.com/huggingface/candle) (pure Rust, no ONNX). No OpenAI API key, no internet
- **MCP server** — `gbrain serve` exposes all 16 tools over stdio JSON-RPC 2.0. Works with Claude Code, any MCP-compatible agent
- **Hybrid search** — FTS5 keyword + vector semantic search with set-union merge, exact-match short-circuit, and optional palace-style hierarchical filtering
- **Progressive retrieval** — token-budget-gated content expansion (summary → section → full page)
- **Temporal knowledge graph** — typed links with validity windows, contradiction detection via assertions
- **Knowledge gap tracking** — agent logs what it can't answer; research skill resolves gaps later
- **Fat skills** — all 8 agent workflows live in markdown SKILL.md files, embedded in the binary and overridable
- **Integrity validation** — `gbrain validate --all` checks links, assertions, and embeddings
- **JSONL streaming** — `gbrain pipe` for shell pipeline automation; `gbrain call` for raw MCP tool invocation

## Tech stack

| Component | Choice |
| --------- | ------ |
| Language | Rust |
| Database | SQLite via `rusqlite` (bundled) |
| Full-text search | FTS5 (built into SQLite) |
| Vector search | `sqlite-vec` (statically linked) |
| Embeddings | `candle` + BGE-small-en-v1.5 (pure Rust, local) |
| CLI | `clap` |
| MCP server | `rmcp` (stdio transport) |

## Quick start

> Phase 3 is complete. Build from source today, use GitHub Release binaries for `v0.9.0`, or try the shell installer during the current test-release cycle.

### Install options

| Method | Status |
| ------ | ------ |
| Build from source (`cargo build --release`) | ✅ Available now |
| GitHub Release binary (macOS ARM/x86, Linux x86_64/ARM64) | ✅ Available — `v0.9.0` test release |
| `npm install -g gbrain` | 🚧 Staged — package and workflow are ready, public publish follows shell-installer testing |
| One-command curl installer | ✅ Available — `curl -fsSL https://raw.githubusercontent.com/macro88/gigabrain/main/scripts/install.sh \| sh` |

**Build from source** is available now. **GitHub Releases** and the **shell installer** are available for the `v0.9.0` test release. The npm package rollout is staged behind that shell-installer validation window.

Install with the shell script:

```bash
curl -fsSL https://raw.githubusercontent.com/macro88/gigabrain/main/scripts/install.sh | sh
```

Download a pre-built binary from GitHub Releases:

```bash
VERSION="v0.9.0"
PLATFORM="darwin-arm64"   # darwin-arm64 | darwin-x86_64 | linux-x86_64 | linux-aarch64
curl -fsSL "https://github.com/macro88/gigabrain/releases/download/${VERSION}/gbrain-${PLATFORM}" -o "gbrain-${PLATFORM}"
curl -fsSL "https://github.com/macro88/gigabrain/releases/download/${VERSION}/gbrain-${PLATFORM}.sha256" -o "gbrain-${PLATFORM}.sha256"
shasum -a 256 --check "gbrain-${PLATFORM}.sha256"
# Option A: install for the current user
mkdir -p "${HOME}/.local/bin"
mv "gbrain-${PLATFORM}" "${HOME}/.local/bin/gbrain"
chmod +x "${HOME}/.local/bin/gbrain"

# Option B: install system-wide (requires root)
sudo install -m 755 "gbrain-${PLATFORM}" /usr/local/bin/gbrain
```

Or build from source:

```bash
git clone https://github.com/macro88/gigabrain
cd gigabrain
cargo build --release
# Binary at target/release/gbrain
```

> **Shell-first rollout.** `v0.9.0` is the install-simplification test release: use the shell installer or GitHub Releases today. npm packaging is implemented, but public publication stays gated until the shell path is proven and `NPM_TOKEN` is configured for release automation.

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

# Start MCP server (stdio)
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

**Phase 1 tools (core read/write):** `brain_get`, `brain_put`, `brain_query`, `brain_search`, `brain_list`

**Phase 2 tools (intelligence layer):** `brain_link`, `brain_link_close`, `brain_backlinks`, `brain_graph`, `brain_check`, `brain_timeline`, `brain_tags`

**Phase 3 tools (gaps, stats, raw data):** `brain_gap`, `brain_gaps`, `brain_stats`, `brain_raw`

All 16 tools are available when you run `gbrain serve`.

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

`person`, `company`, `deal`, `project`, `concept`, `original`, `source`, `media`, `decision`, `commitment`, `action_item`

The `original` type is for your own thinking — distinct from world knowledge. Everything else is compiled external intelligence.

## Contributing

GigaBrain is open for contributions. All three phases have shipped. Phase 3 (`v1.0.0`) is complete.

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

# Release (optimised, ~90MB with embedded model weights)
cargo build --release

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
