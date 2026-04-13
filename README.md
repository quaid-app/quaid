# GigaBrain

> Open-source personal knowledge brain. SQLite + FTS5 + vector embeddings in one file. Thin CLI harness, fat skill files. MCP-ready from day one. Runs anywhere. No API keys, no internet, no Docker. Truly static single binary.

**Status:** `Sprint 0 complete` — repository scaffold, CI, and phase proposals are in place. Phase 1 (Core) has not started. Nothing is implemented yet. [See the roadmap →](#roadmap)

---

Inspired by [Garry Tan's GBrain work](https://gist.github.com/garrytan/49c88e83cf8d7ae95e087426368809cb), GigaBrain adapts the same core concept — a personal knowledge brain with compiled truth and append-only timelines — to a local-first Rust + SQLite architecture built for portable, offline use. No API keys. No internet required. No Docker. One static binary, drop it anywhere.

## Roadmap

GigaBrain is built in explicit phases. Each phase has a hard gate — no phase begins until the previous one is reviewed and merged.

| Phase | Status | What ships |
| ----- | ------ | ---------- |
| **Sprint 0** — Repository scaffold | ✅ Complete | `Cargo.toml`, module stubs, `schema.sql`, skill stubs, CI/CD workflows |
| **Phase 1** — Core storage + CLI | 🔜 Not started | `gbrain init`, `import`, `get`, `put`, `search`, `link`, `graph`, `stats`, `compact` |
| **Phase 2** — Intelligence layer | 🔜 Not started | Local embeddings, hybrid search, MCP server, `query`, `check`, `gaps` |
| **Phase 3** — Polish + release | 🔜 Not started | Benchmarks, cross-compiled binaries, fat skill finalization |

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

## Planned features

> These are the target features for v0.1. None are implemented yet. The spec and OpenSpec proposals define exactly how each will work.

- **Single static binary** — ~90MB including embedded BGE-small-en-v1.5 model weights. Zero runtime dependencies.
- **SQLite everything** — FTS5 full-text search, `sqlite-vec` vector similarity, typed link graph — all in one `brain.db` file
- **Local embeddings** — BGE-small-en-v1.5 via [candle](https://github.com/huggingface/candle) (pure Rust, no ONNX). No OpenAI API key, no internet
- **MCP server** — `gbrain serve` exposes all tools over stdio JSON-RPC 2.0. Works with Claude Code, any MCP-compatible agent
- **Hybrid search** — FTS5 keyword + vector semantic search with set-union merge, exact-match short-circuit, and optional palace-style hierarchical filtering
- **Progressive retrieval** — token-budget-gated content expansion (summary → section → full page)
- **Temporal knowledge graph** — typed links with validity windows, contradiction detection via assertions
- **Knowledge gap tracking** — agent logs what it can't answer; research skill resolves gaps later
- **Fat skills** — all agent workflows live in markdown SKILL.md files, embedded in the binary and overridable

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

> **Not yet available.** GigaBrain is pre-implementation — Sprint 0 (scaffold) is complete, but Phase 1 (Core) has not started. The release install instructions below describe what will ship at v0.1.

Once Phase 3 ships, install from a GitHub release:

```bash
VERSION="v0.1.0"
PLATFORM="darwin-arm64"   # darwin-arm64 | darwin-x86_64 | linux-x86_64 | linux-aarch64
curl -fsSL "https://github.com/macro88/gigabrain/releases/download/${VERSION}/gbrain-${PLATFORM}" -o gbrain
curl -fsSL "https://github.com/macro88/gigabrain/releases/download/${VERSION}/gbrain-${PLATFORM}.sha256" -o gbrain.sha256
echo "$(cat gbrain.sha256)  gbrain" | shasum -a 256 --check
cp gbrain /usr/local/bin/gbrain && chmod +x /usr/local/bin/gbrain
```

Or build from source (the scaffold compiles today; full implementation begins in Phase 1):

```bash
git clone https://github.com/macro88/gigabrain
cd gigabrain
cargo build --release
# Binary at target/release/gbrain (~90MB with embedded model weights once Phase 2 ships)
```

---

## Usage

> **Planned API.** These commands are the target surface for Phase 1 and Phase 2. They reflect the spec exactly but are not yet implemented. See [`docs/spec.md`](docs/spec.md) for full command signatures.

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

Available tools: `brain_query`, `brain_search`, `brain_get`, `brain_put`, `brain_ingest`, `brain_link`, `brain_link_close`, `brain_backlinks`, `brain_graph`, `brain_timeline`, `brain_tags`, `brain_list`, `brain_check`, `brain_gap`, `brain_gaps`, `brain_stats`, `brain_raw`.

## Skills

Skills are markdown files that tell agents how to use GigaBrain. They live in `skills/` and are embedded in the binary by default, extracted to `~/.gbrain/skills/` on first run. Drop a custom `SKILL.md` in your working directory to override any default.

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
gbrain skills doctor
```

## Page types

`person`, `company`, `deal`, `project`, `concept`, `original`, `source`, `media`, `decision`, `commitment`, `action_item`

The `original` type is for your own thinking — distinct from world knowledge. Everything else is compiled external intelligence.

## Contributing

GigaBrain is actively looking for contributors as Phase 1 begins.

**How we work:**

1. **Read the spec first.** [`docs/spec.md`](docs/spec.md) is the authoritative design document — schema, algorithms, CLI surface, skill design, benchmarks. Everything is defined there.
2. **Proposals before code.** Every meaningful change (code, docs, tests, benchmarks) requires an OpenSpec change proposal in [`openspec/changes/`](openspec/changes/) following the instructions in [`openspec/`](openspec/). This is the design record before implementation.
3. **Check the phase proposals.** The four phase proposals in `openspec/changes/` define the full scope and acceptance criteria for each phase. Pick up work that aligns with the current active phase.
4. **CI gates everything.** PRs must pass `cargo check` + `cargo test` before review.

**Good first contributions for Sprint 0 / Phase 1 prep:**
- Review the [`sprint-0-repo-scaffold`](openspec/changes/sprint-0-repo-scaffold/proposal.md) and [`p1-core-storage-cli`](openspec/changes/p1-core-storage-cli/proposal.md) proposals and raise questions in issues
- Improve skill stub content in [`skills/`](skills/)
- Add fixture pages to [`tests/fixtures/`](tests/fixtures/)

**Ground rules:**
- No implementation PRs until the Sprint 0 PR merges to `main`
- Keep PRs scoped to a single phase concern
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
