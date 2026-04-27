# Quaid vs QMD: Friction Analysis and Path to Replacing QMD for OpenClaw Users

**Date:** 2026-04-18  
**Author:** Doug Aillm  
**Purpose:** Identify why QMD wins on daily friction, and what Quaid needs to close the gap  

---

## The Core Problem

QMD feels instant. Quaid feels like effort. That gap is almost entirely architectural - not quality.

I've been running both for weeks. Quaid's search quality is genuinely better for conceptual queries. But I reach for QMD every time because it requires zero setup per session. Until that changes, OpenClaw users will default to QMD out of habit.

---

## How QMD's "Always-On" Index Actually Works

This is the key thing to understand before designing the fix.

QMD is **not** a daemon. It has no background process. The "always-on" feel is an illusion created by two things:

1. **Persistent SQLite index** at `~/.cache/qmd/index.sqlite` (41.5 MB for my vault). Built once, stored forever, updated incrementally.

2. **Poll-based updates via cron** - I run `qmd update` every hour. On each run, it does a full glob scan of the collection root, SHA-256 hashes every file, skips if hash matches what's in the DB, updates only changed files. On a 356-file vault with no changes, this takes about 3 seconds.

The search command (`qmd search "query"`) hits the pre-built FTS5 index and sqlite-vec embeddings directly. No server. No startup. The binary reads the SQLite file, runs the query, exits. Total wall time: under 1 second for most queries.

**This is the architectural advantage QMD has:** serverless reads from a persistent index.

Quaid requires `quaid serve` to be running for MCP/API access. For CLI-only use, `quaid search` works without a server - but users don't know that, and the docs don't lead with it.

---

## Friction Audit: Side-by-Side

| Task | QMD | Quaid |
|------|-----|-----------|
| Search vault (first use) | `qmd search "query"` - 1 cmd | `quaid import ~/docs && quaid serve` then query |
| Search vault (returning user) | `qmd search "query"` - instant | Need serve running, or use CLI directly |
| Keep index fresh | Cron runs `qmd update` hourly - zero thought | Manual import or cron setup |
| Add to OpenClaw context | Built-in skill, auto-configured | Manual MCP config, `quaid serve` must be running |
| Cross-session availability | Always available (SQLite file persists) | Only if serve process survived |
| Embedding updates | `qmd embed` - separate step, takes ~30s | Part of import, but slow (427s for 356 files) |

The biggest friction point is the import/serve split. QMD users never think about "did I import today?" Quaid users have to.

---

## Root Cause Analysis

### 1. The Serve Requirement

`quaid serve` is required for MCP integration, which is Quaid's primary value proposition for OpenClaw. But the MCP story means users think of Quaid as a server they need to run, not a tool they query. QMD has no equivalent - it's always queryable because it's stateless.

**Fix:** Auto-start `quaid serve` as a background daemon on first use, or make `quaid init` optionally register it as a launchd/systemd service. One-time setup, then it's always there.

### 2. Import Speed Regression

v0.9.4 import time: 427 seconds for 356 files.  
v0.9.1 import time: 159 seconds for the same corpus.  
QMD update time (no changes): 3 seconds. QMD update time (full re-index): ~45 seconds.

Until import speed reaches parity with v0.9.1, the daily workflow is painful. Users who import once and forget it will hit stale results. Users who import daily will hate it.

**Fix:** Track file mtimes or content hashes at import time. Skip unchanged files entirely (same mechanism QMD uses). This should bring re-import time down to under 10 seconds for a warm vault.

### 3. No Incremental Update Path

QMD has a clean incremental update model: on each `qmd update`, it only re-indexes files whose SHA-256 hash changed. New files get indexed. Deleted files get tombstoned. Changed files get a new content record. The FTS5 and embedding indexes update accordingly.

Quaid has `quaid import` which appears to do a full re-import each time. There's no equivalent of `qmd update` that cheaply syncs changes.

**Fix:** Add `quaid sync` (or `quaid import --incremental`) that mirrors QMD's approach. Hash-compare on mtime, only re-embed changed content. This is the single highest-impact change for daily usability.

### 4. The OpenClaw Integration Gap

QMD has a dedicated OpenClaw skill that wires it into the agent context automatically. When I run a search during a session, I get results in my context without any manual setup.

Quaid's MCP integration requires:
- `quaid serve` running (see problem 1)
- Manual MCP config pointing to the serve endpoint
- OpenClaw configured to call that MCP server

That's 3 setup steps vs 0. Most users won't complete all three.

**Fix:** Ship an OpenClaw skill for Quaid that mirrors the QMD skill. The skill should auto-start `quaid serve` if it's not running, auto-configure the MCP endpoint, and provide a `gmemory_search` function that the agent can call directly. The skill handles all the plumbing.

### 5. Cold Start Psychology

When a user sees `qmd search "test"` return results in 800ms, they trust it. When they see `quaid serve` as a prerequisite, their brain registers "this is a service I need to manage." That mental model kills adoption.

The fix isn't just technical - it's UX. Quaid needs to feel like a tool you query, not a service you operate.

---

## Specific Recommendations for the Squad

### P0 - Must have to compete with QMD

**1. Incremental import (`quaid sync` or `--incremental` flag)**  
Hash-check files on import. Skip unchanged. Re-import only modified/new files. Target: re-sync a 400-file vault with no changes in under 5 seconds.

**2. Auto-start daemon option**  
`quaid daemon install` or `quaid init --daemon` that registers `quaid serve` as a launchd service (macOS) / systemd unit (Linux). One command, then it's always running. No manual start required.

**3. OpenClaw skill that handles the full setup**  
Ship `skills/openclaw/SKILL.md` that covers: installing quaid, running `quaid import`, starting the daemon, configuring MCP in openclaw.json. Zero manual steps for OpenClaw users after installing the skill.

### P1 - Significant quality-of-life improvements

**4. Fix import speed regression (v0.9.2 introduced 2.7x slowdown)**  
Already filed as issue #59. Still unresolved in v0.9.4. This is blocking daily use.

**5. `quaid status` command**  
Show: DB path, file count, last import time, serve status, embedding coverage. Mirrors `qmd status`. Users need a quick way to verify their setup is healthy.

**6. Cron-friendly `quaid sync` exit codes**  
Return 0 if nothing changed, 1 if files were updated, 2 on error. Lets users build smart cron jobs that only do expensive work (embedding re-generation) when content actually changed.

### P2 - Long-term differentiation

**7. Hybrid search with explicit BM25/semantic weighting**  
QMD exposes `--hybrid-weight` on search. Quaid should too. Different query types need different balances.

**8. Collection support**  
QMD indexes multiple collections (docs, memory, etc.) and scopes searches to them. Quaid uses a single memory.db. For multi-vault users (personal notes + work notes + agent memory), this matters.

**9. Watch mode**  
`quaid sync --watch` using FSEvents (macOS) or inotify (Linux). Real-time index updates without cron. This is the "always-on" feel that makes QMD so frictionless - except Quaid can do it properly with a file watcher rather than polling.

---

## What Quaid Already Does Better

To be clear: this report is about friction, not quality. Quaid wins on:

- **Hybrid search quality** - FTS5 + vector in one query, properly weighted. QMD does this too but Quaid's results feel more precise for conceptual queries.
- **MCP integration** - When it's running, the MCP surface is richer than anything QMD offers. Agents can query with structure, not just keyword search.
- **Contradiction detection** - No equivalent in QMD. This is genuinely novel for knowledge management.
- **Compiled binary** - Faster startup than QMD's Bun runtime. Sub-100ms for simple queries vs ~300ms for QMD.
- **Single-file DB** - memory.db is portable in a way QMD's index.sqlite isn't. Copy the file, your brain moves with it.

The architecture is right. The daily workflow friction is solvable. The gap isn't fundamental - it's a few specific missing features.

---

## What Would Make Me Actually Love It

Three things. In order of impact.

**1. Zero-friction daily sync**

This is the big one. Right now importing takes 7 minutes. That's not a tool I reach for - that's a project I schedule. QMD re-syncs my whole vault in under 10 seconds because it skips files that haven't changed. Until Quaid does the same, I'll keep defaulting to QMD for quick lookups. Fix `quaid sync` to hash-check files and skip unchanged ones, and the daily friction almost disappears overnight.

**2. Tell me something I didn't know**

QMD is search. It finds what I ask for. Quaid should be discovery - surfacing what I forgot I knew, or what contradicts something I believe, or connections I'd never make manually. The contradiction detection is the seed of this. But it needs to go further:

- Weekly "your vault has these gaps" report - topics you've touched but never finished
- "This note contradicts something you wrote 3 months ago" alerts  
- Semantic clustering - "you have 14 notes that are all really about the same thing, consider merging"

That's the killer feature QMD can't do. That's what earns a permanent slot in my workflow.

**3. Stay running without me thinking about it**

`quaid serve` dying silently when I close a terminal is a silent workflow killer. I don't notice until I need it. A single command to register it as a macOS launch daemon - `quaid daemon install` - and I never think about it again. Same energy as Postgres.app or Ollama. It's just always there.

---

## Bottom Line

QMD wins on friction because it's designed around "query anywhere, anytime." Quaid wins on quality but loses on setup friction. Close the gap with incremental sync + auto-daemon + an OpenClaw skill, and Quaid becomes the obvious choice for OpenClaw users. Right now, it's the obvious choice for power users who'll tolerate setup friction. That's a much smaller audience.

Quaid's hybrid search quality is already noticeably better than QMD for conceptual queries. "What do I know about validator economics" returns something useful. QMD returns exact matches. That difference is real and worth building on. The quality is there. The friction isn't gone yet.

The two-line pitch for the squad: **make `quaid sync` work like `qmd update`, and make `quaid serve` start itself.** Everything else is polish.

---

*Filed for squad evaluation. Happy to run follow-up DAB benchmarks on any of these improvements once implemented.*
