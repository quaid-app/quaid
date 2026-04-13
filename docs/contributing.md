# Contributing to GigaBrain

Welcome. This guide covers everything a new contributor needs to navigate the codebase, understand how work is organised, and make a meaningful first contribution.

---

## What GigaBrain is

GigaBrain is a local-first personal knowledge brain: a single Rust binary (~90MB including embedded model weights) that wraps SQLite + FTS5 + local vector embeddings. It stores structured knowledge pages, searches them with hybrid keyword + semantic queries, and exposes an MCP server for any AI agent client.

Read [getting-started.md](getting-started.md) first if you haven't. Read [spec.md](spec.md) for the full technical specification.

---

## Repository layout

```
gigabrain/
├── src/
│   ├── main.rs               # CLI entrypoint (clap dispatch)
│   ├── schema.sql            # Full v4 DDL (embedded via include_str!)
│   ├── core/                 # Library modules
│   │   ├── db.rs             # SQLite connection, schema init, WAL, sqlite-vec
│   │   ├── types.rs          # All structs
│   │   ├── markdown.rs       # Frontmatter parse, compiled-truth/timeline split
│   │   ├── fts.rs            # FTS5 search, BM25 scoring
│   │   ├── inference.rs      # Candle init, BGE-small embeddings, vector search
│   │   ├── search.rs         # Hybrid: SMS + FTS5 + vector + set-union merge
│   │   ├── progressive.rs    # Token-budget-gated content expansion
│   │   ├── palace.rs         # Wing/room derivation for palace filtering
│   │   ├── novelty.rs        # Jaccard + cosine dedup
│   │   ├── migrate.rs        # import_dir / export_dir / validate_roundtrip
│   │   ├── chunking.rs       # Temporal sub-chunking
│   │   └── graph.rs          # Graph neighbourhood traversal (Phase 2)
│   ├── commands/             # One file per CLI command
│   │   ├── init.rs, get.rs, put.rs, list.rs, stats.rs
│   │   ├── search.rs, query.rs, embed.rs
│   │   ├── import.rs, export.rs, ingest.rs
│   │   ├── link.rs, graph.rs, timeline.rs
│   │   ├── gaps.rs, check.rs
│   │   ├── serve.rs, compact.rs, config.rs, version.rs
│   │   └── tags.rs
│   └── mcp/
│       └── server.rs         # MCP stdio server (JSON-RPC 2.0 via rmcp)
├── skills/                   # Fat markdown skill files
│   ├── ingest/SKILL.md
│   ├── query/SKILL.md
│   ├── maintain/SKILL.md
│   ├── briefing/SKILL.md
│   ├── research/SKILL.md
│   ├── enrich/SKILL.md
│   ├── alerts/SKILL.md
│   └── upgrade/SKILL.md
├── tests/
│   └── fixtures/             # Sample markdown pages for integration tests
├── benchmarks/               # Benchmark harness and results
├── docs/                     # User-facing documentation (you are here)
│   ├── spec.md               # Full technical specification
│   ├── getting-started.md    # New user / new contributor onboarding
│   ├── roadmap.md            # Phased delivery plan and status
│   └── contributing.md       # This file
├── openspec/                 # Structured change proposals
│   └── changes/              # One directory per proposal
├── Cargo.toml
├── AGENTS.md                 # Agent instructions (read by any AI spawned here)
└── CLAUDE.md                 # Extended context for Claude-family agents
```

---

## Build and test

```bash
# Check that everything compiles (fast; no linking)
cargo check

# Debug build
cargo build

# Release build (~90MB with embedded model weights)
cargo build --release

# Run tests
cargo test

# Cross-compile for fully static Linux binary
cargo install cross
cross build --release --target x86_64-unknown-linux-musl
```

CI runs `cargo check` and `cargo test` on every pull request. Both must pass before a PR can merge.

---

## Tech stack

| Component | Crate | Notes |
| --------- | ----- | ----- |
| CLI | `clap` (derive) | One `mod` per command in `src/commands/` |
| Database | `rusqlite` (bundled) | SQLite compiled into the binary |
| Full-text search | FTS5 | Built into SQLite |
| Vector search | `sqlite-vec` | Statically linked extension |
| Embeddings | `candle` + BGE-small-en-v1.5 | Pure Rust, no ONNX runtime |
| MCP server | `rmcp` | stdio transport, JSON-RPC 2.0 |
| Serialisation | `serde` + `serde_json` + `serde_yaml` | |
| Markdown | `pulldown-cmark` | |
| Error handling | `anyhow` + `thiserror` | |
| Async | `tokio` | Required by `rmcp` |

---

## How changes are proposed

GigaBrain uses **OpenSpec** for structured change proposals. The rule is simple:

> Every meaningful code, docs, or architecture change requires an OpenSpec proposal in `openspec/changes/` _before_ implementation begins.

**To propose a change:**

1. Create a new directory under `openspec/changes/` named for your change (e.g., `openspec/changes/my-feature/`).
2. Write a `proposal.md` following the local instructions in `openspec/` — include `id`, `title`, `status: proposed`, scope, non-goals, and success criteria.
3. Share it with the team for review before writing any implementation code.

This keeps design intent visible and reviewable before any work starts.

---

## Workflow overview

1. **Spec first.** Check `docs/spec.md` for the canonical design. If your change is not in the spec, propose it via OpenSpec.
2. **OpenSpec proposal.** Write a proposal and get it reviewed.
3. **Branch.** Branch off `main` — name it `phase-N/short-description` or `fix/short-description`.
4. **Implement.** Follow the module structure above. One command per file in `src/commands/`. Core logic in `src/core/`.
5. **Test.** Unit tests live next to the code they test (Rust convention). Integration tests go in `tests/`.
6. **CI.** Push — CI runs `cargo check` and `cargo test` automatically.
7. **PR.** Open a pull request. Link the OpenSpec proposal. Get reviewer sign-off per the phase gates.

---

## Sprint 0 operational setup

Before Phase 1 implementation can begin, three one-time repository setup tasks must be completed. These steps require write access to the GitHub repository and a working `gh` CLI authenticated to the repo.

**Recommended order:** create labels and issues first, then push the Sprint 0 branch, then open the PR linking the new issue and the relevant OpenSpec proposals.

### 1. Create GitHub labels and issues

Use the helper script from the repo root:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\create-labels-and-issues.ps1
```

If you need to run the steps manually instead, create labels first:

```powershell
gh label create "phase-1"  --color "0075ca" --description "Phase 1: Core Storage, CLI, Search, MCP"
gh label create "phase-2"  --color "e4e669" --description "Phase 2: Intelligence Layer"
gh label create "phase-3"  --color "d93f0b" --description "Phase 3: Polish, Benchmarks, Release"
gh label create "squad:fry"       --color "bfd4f2" --description "Assigned to Fry (main engineer)"
gh label create "squad:bender"    --color "bfd4f2" --description "Assigned to Bender (tester)"
gh label create "squad:professor" --color "bfd4f2" --description "Assigned to Professor (code reviewer)"
gh label create "squad:nibbler"   --color "bfd4f2" --description "Assigned to Nibbler (adversarial reviewer)"
gh label create "squad:kif"       --color "bfd4f2" --description "Assigned to Kif (benchmarks)"
gh label create "squad:zapp"      --color "bfd4f2" --description "Assigned to Zapp (DevRel/release)"
```

Then create the issues:

```powershell
gh issue create --title "[Sprint 0] Repository scaffold + CI/CD" --body "Tracks the Sprint 0 scaffold work. OpenSpec: openspec/changes/sprint-0-repo-scaffold/proposal.md" --label "squad:fry"
gh issue create --title "[Phase 1] Core storage, CLI, search, MCP" --body "Fry implements Phase 1. OpenSpec: openspec/changes/p1-core-storage-cli/proposal.md. Gate: round-trip tests pass; MCP connects; static binary verified." --label "squad:fry,phase-1"
gh issue create --title "[Phase 1] Round-trip test + ship gate sign-off" --body "Bender validates the Phase 1 ship gate before Phase 2 can begin." --label "squad:bender,phase-1"
gh issue create --title "[Phase 1] Code review: db.rs, search.rs, inference.rs" --body "Professor reviews the three critical Phase 1 modules." --label "squad:professor,phase-1"
gh issue create --title "[Phase 1] Adversarial review: MCP server" --body "Nibbler adversarially reviews the MCP server for OCC enforcement and injection risks." --label "squad:nibbler,phase-1"
gh issue create --title "[Phase 2] Intelligence layer" --body "Fry implements Phase 2. OpenSpec: openspec/changes/p2-intelligence-layer/proposal.md. Blocked until Phase 1 gate passes." --label "squad:fry,phase-2"
gh issue create --title "[Phase 3] Benchmarks + release gates" --body "Kif establishes BEIR baseline and all release gates. OpenSpec: openspec/changes/p3-polish-benchmarks/proposal.md" --label "squad:kif,phase-3"
gh issue create --title "[Phase 3] v0.1.0 release" --body "Zapp coordinates the v0.1.0 GitHub Release after Phase 3 gates pass." --label "squad:zapp,phase-3"
```

> **Note:** Labels must exist before issue creation — `gh issue create --label` silently ignores labels that do not exist yet.

### 2. Create the Sprint 0 branch and push the scaffold

Use the helper script from the repo root:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\push-sprint0-branch.ps1
```

If you need to run the steps manually instead:

```powershell
Set-Location C:\path\to\gigabrain

git checkout -b sprint-0/scaffold
git add .
git commit -m "Sprint 0: repository scaffold, CI/CD, OpenSpec proposals

- Cargo.toml with full dependency declarations
- src/ module stubs (commands, core, mcp)
- src/schema.sql — full v4 DDL
- skills/ stubs for all 8 skills
- tests/fixtures, benchmarks/README.md
- CLAUDE.md, AGENTS.md
- .github/workflows/ci.yml and release.yml
- openspec/changes/ proposals for all 4 phases
- .squad/ team configuration and decisions

Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"

git push origin sprint-0/scaffold
```

### 3. Open the scaffold PR

Open a PR on GitHub targeting `main`. Use this PR body:

```
## Sprint 0: Repository Scaffold

### What
Full repository scaffold for GigaBrain v0.1.0 as specified in
`openspec/changes/sprint-0-repo-scaffold/proposal.md`.

### OpenSpec Reference
- openspec/changes/sprint-0-repo-scaffold/proposal.md
- openspec/changes/p1-core-storage-cli/proposal.md

### Gate
`cargo check` must pass before merge. Phase 1 implementation begins
after this PR merges.
```

---

## Phase reviewer gates

Each phase has designated reviewers before it can ship:

| Reviewer | Responsibilities |
| -------- | ---------------- |
| Professor | Code review on `db.rs`, `search.rs`, `inference.rs` |
| Nibbler | Adversarial review on MCP server (OCC enforcement, injection safety) |
| Bender | End-to-end round-trip validation sign-off |
| Scruffy | Unit test coverage on markdown parser and search merge logic |

---

## Constraints to keep in mind

- **Single writer.** No auth, no RBAC, no multi-tenant. Do not add locking abstractions that assume multi-writer access.
- **Optimistic concurrency on MCP writes.** `brain_put` requires `expected_version`. Always re-fetch before writing.
- **Ingest idempotency.** SHA-256 of source content is the idempotency key. Re-ingesting the same content must be a no-op.
- **Static binary.** Every dependency must be statically linkable. No dynamic `.so` / `.dylib` dependencies at runtime.
- **No internet at runtime.** Embeddings run locally via candle. Do not add network calls to the hot path.

---

## Skills are not code

Skills live in `skills/*/SKILL.md` — plain markdown. They tell agents _how_ to use GigaBrain, not what it does. If you are changing workflow behaviour, update the relevant SKILL.md. If you are changing what the binary can do, update `src/` and `docs/spec.md`.

To override a default skill locally, drop a `SKILL.md` in your working directory. The binary will prefer it over the embedded default.

---

## Getting help

- Full technical spec: [docs/spec.md](spec.md)
- Phased delivery plan: [docs/roadmap.md](roadmap.md)
- Agent instructions: [AGENTS.md](../AGENTS.md) and [CLAUDE.md](../CLAUDE.md)
- Open an issue on GitHub for bugs or feature requests.
