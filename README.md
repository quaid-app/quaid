<div align="center">
  <h1>Quaid</h1>
  <p><strong>Local-first persistent memory for AI agents.<br>SQLite + FTS5 + vector search. No cloud. No API key. Just memory that works.</strong></p>

  <a href="https://github.com/quaid-app/quaid/actions/workflows/ci.yml"><img src="https://github.com/quaid-app/quaid/workflows/CI/badge.svg" alt="CI"></a>
  <a href="https://github.com/quaid-app/quaid/releases/latest"><img src="https://img.shields.io/github/v/release/quaid-app/quaid" alt="Release"></a>
  <a href="https://github.com/quaid-app/quaid/issues/220"><img src="https://img.shields.io/badge/DAB%20v1.0-140%2F200%20(70%25)-orange" alt="DAB v1.0 140/200"></a>
  <a href="https://github.com/quaid-app/quaid/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License"></a>

  <br><br>

  <a href="#quick-start">Quick Start</a> •
  <a href="https://quaid.app">quaid.app</a> •
  <a href="https://quaid-app.github.io/quaid-evals">Benchmarks</a> •
  <a href="https://github.com/quaid-app/quaid/discussions">Discussions</a>
</div>

---

## Why Quaid?

Most agent memory systems require a cloud service, a running database, or an API key. Quaid runs on your machine:

- **Local-first** — single SQLite file, no cloud dependency, works offline
- **PARA-native** — organizes memory as a knowledge base (Projects, Areas, Resources, Archives), not a flat list of facts
- **24 MCP tools, knowledge-graph path output, daemon runtime, `quaid daemon` lifecycle commands, `quaid status`, and an opt-in HTTP/SSE MCP transport in the current published release (`v0.22.6`)**
- **Hybrid retrieval** — FTS5 full-text + local BGE vector embeddings, combined via RRF
- **Verified by benchmarks** — [140/200 (70%) on DAB v1.0](https://github.com/quaid-app/quaid/issues/220), P@5 on MSMARCO ahead of BM25 baseline

---

## Development Benchmarks

Fast feedback loop for iterative development. **No dependencies required** — corpus is generated automatically on first run.

### Quick start

```bash
make bench          # build + 20 queries (~30s on M1/M3)
make bench-no-build # query-only, no rebuild (~1s)
make bench-setup    # force corpus rebuild
```

### What it measures

20 queries (10 FTS + 10 semantic) against a 60-page synthetic corpus covering: agent memory, DeFi, stablecoin regulation, Rust performance, vector search, knowledge graphs, smart contracts, cross-chain bridges, zero-knowledge proofs, and RAG.

Output shows pass/fail per query with delta from last run for instant regression detection.

### Score guide

| Score | Grade |
|-------|-------|
| 14–20 | ✅ Good |
| 10–13 | ⚠️  Watch for regressions |
| < 8   | 🔴 Regression gate (exits 1) |

---

## Quick Start

```bash
# Install
curl -fsSL https://raw.githubusercontent.com/quaid-app/quaid/main/scripts/install.sh | sh

# Initialize
quaid init ~/.quaid/memory.db

# Add your knowledge base
quaid collection add docs ~/Documents/notes

# Generate embeddings
quaid embed

# Search
quaid search "how did we handle auth"
quaid query "what decisions did we make last week"
```

---

## MCP Setup (Claude Code / Cursor / Windsurf)

Add to your `.mcp.json`:

```json
{
  "mcpServers": {
    "quaid": {
      "command": "quaid",
      "args": ["serve"],
      "env": {
        "QUAID_DB": "/Users/you/.quaid/memory.db"
      }
    }
  }
}
```

The current published release (`v0.22.6`) exposes 24 MCP tools over stdio, graph path output, and an opt-in HTTP/SSE transport via `quaid serve --http` or `quaid daemon run --http`.

---

## Install

### One-line installer (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/quaid-app/quaid/main/scripts/install.sh | sh
```

Sets up `PATH` and `QUAID_DB` automatically. Use `QUAID_CHANNEL=online` for the smaller binary that downloads embeddings on first use.

### Download a binary

```bash
VERSION="<published-tag>"   # for example: v0.22.6
PLATFORM="darwin-arm64"   # darwin-arm64 | darwin-x86_64 | linux-x86_64 | linux-aarch64
curl -fsSL "https://github.com/quaid-app/quaid/releases/download/${VERSION}/quaid-${PLATFORM}-online" \
  -o quaid && chmod +x quaid && sudo mv quaid /usr/local/bin/
```

Use a published tag here. The [latest GitHub Release](https://github.com/quaid-app/quaid/releases/latest) includes the knowledge graph layer, daemon runtime, and HTTP/SSE transport.

### Build from source

```bash
git clone https://github.com/quaid-app/quaid
cd quaid
./scripts/setup-git-hooks.sh   # blocks direct pushes to main/master for this clone
cargo build --release
```

On Windows PowerShell, run `powershell -ExecutionPolicy Bypass -File .\scripts\setup-git-hooks.ps1` after `cd quaid`.

---

## How it works

Two ideas borrowed from [Garry Tan's compiled knowledge model](https://gist.github.com/garrytan/49c88e83cf8d7ae95e087426368809cb):

**Compiled truth (above the line)** — always current, rewritten when new information arrives. What we know now.

**Timeline (below the line)** — append-only, never rewritten. What happened and when.

Every page in Quaid has both. Agents read and write through Quaid's MCP surface via stdio — 24 tools in the current published release (`v0.22.6`) — with no REST API and no network dependency. An opt-in HTTP/SSE transport is also available via `quaid serve --http` or `quaid daemon run --http`.

**Hybrid retrieval:** FTS5 keyword search for exact recall (names, slugs, tags) combined with local BGE vector embeddings for semantic search. Set-union merge, exact-match short-circuit.

**Thin harness, fat skills:** The binary is plumbing. The intelligence lives in `SKILL.md` files that agents read at session start — swap or extend without rebuilding.

---

## Core commands

```bash
# Initialize a new memory database
quaid init ~/.quaid/memory.db

# Attach a live-sync collection (Obsidian vault, etc.)
quaid collection add work ~/Documents/Obsidian

# Ingest a single markdown file directly
quaid ingest /path/to/note.md

# Generate or refresh embeddings
quaid embed
quaid embed --stale --batch-size 32

# Full-text search
quaid search "stablecoin regulation"

# Semantic / hybrid query
quaid query "who has worked with Jensen Huang?"

# Read / write a page
quaid get people/pedro-franceschi
cat updated.md | quaid put people/pedro-franceschi

# Create a typed link between pages
quaid link people/pedro-franceschi companies/brex \
  --relationship founded --valid-from 2017-01-01

# Graph neighbourhood
quaid graph people/pedro-franceschi --depth 2

# Contradiction detection
quaid check --all

# Knowledge gaps
quaid gaps

# Start MCP server (stdio, default)
quaid serve

# Start MCP server with opt-in HTTP/SSE transport
quaid serve --http --port 3112 --trust-loopback

# Install background daemon (macOS launchd or Linux systemd)
quaid daemon install
quaid daemon install --http --port 3112 --trust-loopback   # daemon + HTTP/SSE

# Daemon lifecycle
quaid daemon start
quaid daemon stop
quaid daemon status
quaid daemon logs

# Process-level status overview (daemon, transports, activity)
quaid status
```

---

## MCP tools

The current published release (`v0.22.6`) exposes 24 MCP tools over stdio, plus graph path output, daemon runtime, `quaid daemon` commands, `quaid status`, and opt-in HTTP/SSE transport:

| Category | Tools |
|----------|-------|
| **Core read/write** | `memory_get`, `memory_put`, `memory_query`, `memory_search`, `memory_list` |
| **Conversation workflows** | `memory_add_turn`, `memory_close_session`, `memory_close_action`, `memory_correct`, `memory_correct_continue` |
| **Intelligence** | `memory_link`, `memory_link_close`, `memory_backlinks`, `memory_graph`, `memory_check`, `memory_timeline`, `memory_tags` |
| **Gaps + stats** | `memory_gap`, `memory_gaps`, `memory_stats`, `memory_raw` |
| **Collections + namespaces** | `memory_collections`, `memory_namespace_create`, `memory_namespace_destroy` |

---

## Embedding models

| Alias | Model | Dimensions | Size | Notes |
|-------|-------|-----------|------|-------|
| `small` | BGE-small-en-v1.5 | 384 | 130 MB | Default, fastest |
| `base` | BGE-base-en-v1.5 | 768 | 438 MB | Better recall on larger corpora |
| `large` | BGE-large-en-v1.5 | 1024 | 1.34 GB | Highest recall |
| `m3` | BGE-m3 | 1024 | 2.27 GB | Multilingual |

```bash
QUAID_MODEL=large quaid query "stablecoin regulation"
```

**Airgapped vs online:** `airgapped` binaries embed BGE-small for fully offline use. `online` binaries download and cache the selected model on first semantic use.

---

## Benchmarks

Retrieval quality is verified by [quaid-evals](https://github.com/quaid-app/quaid-evals) — automated benchmarks that run on every release.

| Benchmark | Score | Reference |
|-----------|-------|-----------|
| DAB v1.0 (FTS + semantic + MCP) | **140/200 (70%)** on `v0.22.6` | [#220](https://github.com/quaid-app/quaid/issues/220) |
| MSMARCO P@5 | — | GBrain: 49.1% |
| LoCoMo | — | Mem0: 91.6% |

DAB v1.0 rescored the suite out of 200 points; scores published before v1.0
(such as 193/215) used the original 215-point rubric and are not directly
comparable.

[View full benchmark history →](https://quaid-app.github.io/quaid-evals)

---

## Skills

Skills are markdown files (`SKILL.md`) that tell agents how to use Quaid. Embedded in the binary, extracted to `~/.quaid/skills/` on first run. Drop a custom `SKILL.md` in your working directory to override any default.

| Skill | Purpose |
|-------|---------|
| `ingest` | Meeting transcripts, articles, documents |
| `query` | Search and synthesis workflows |
| `maintain` | Lint, contradiction detection, orphan pages |
| `enrich` | External API enrichment |
| `briefing` | Daily briefing compilation |
| `alerts` | Interrupt-driven notifications |
| `research` | Knowledge gap resolution |
| `upgrade` | Agent-guided updates |

---

## Page types

`person`, `company`, `deal`, `project`, `concept`, `original`, `source`, `media`, `decision`, `commitment`, `action_item`, `area`, `resource`, `archive`, `journal`

PARA folder inference: `1. Projects` → `project`, `2. Areas` → `area`, `3. Resources` → `resource`, `4. Archives` → `archive`. Frontmatter `type:` always wins.

---

## Release channels

| Channel | Binary | Embedding model |
|---------|--------|----------------|
| `airgapped` | ~130 MB | BGE-small embedded, works offline |
| `online` | ~20 MB | Downloads + caches selected model on first use |

---

## Contributing

Read [`openspec/specs/`](openspec/specs/) and [`src/schema.sql`](src/schema.sql) first — together they are the authoritative design record ([`docs/spec.md`](docs/spec.md) is historical). Every meaningful change needs an OpenSpec proposal in [`openspec/changes/`](openspec/changes/) before implementation.

**Good first contributions:**
- Improve skill content in [`skills/`](skills/)
- Add fixture pages to [`tests/fixtures/`](tests/fixtures/)
- Contribute benchmark results (LongMemEval, LoCoMo, BEAM) from your environment

CI must pass (`cargo check` + `cargo test`) before review.

---

## Tech stack

| Component | Choice |
|-----------|--------|
| Language | Rust |
| Database | SQLite + rusqlite (bundled) |
| Full-text | FTS5 |
| Vectors | sqlite-vec (statically linked) |
| Embeddings | candle + BGE family (pure Rust, local) |
| MCP | rmcp (stdio JSON-RPC 2.0) |
| CLI | clap |

---

## Acknowledgements

Inspired by [Garry Tan's compiled knowledge model](https://gist.github.com/garrytan/49c88e83cf8d7ae95e087426368809cb). Quaid adapts the same architecture — SQLite + FTS5 + vectors + MCP + fat skills — in Rust for a fully local, zero-dependency binary.

Techniques from [MemPalace](https://arxiv.org/abs/2410.10674), [OMNIMEM](https://arxiv.org/abs/2406.16026), and [agentmemory](https://github.com/AgentOps-AI/agentmemory).

---

## License

MIT
