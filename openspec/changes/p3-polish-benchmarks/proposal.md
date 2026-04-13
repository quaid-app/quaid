---
id: p3-polish-benchmarks
title: "Phase 3: Polish, Benchmarks, and Release Gates"
status: proposed
type: feature
phase: 3
owner: fry
reviewers: [leela, kif, bender, scruffy, zapp]
created: 2026-04-13
depends_on: p2-intelligence-layer
---

# Phase 3: Polish, Benchmarks, and Release Gates

## What

Complete the skill suite, wire up the full benchmark harness, and ship v0.1.0.

### Skills
- Briefing skill with "what shifted" report
- Alerts skill (interrupt-driven notifications)
- Research skill (knowledge gap resolution via `brain_gap` / `brain_gaps`)
- Upgrade skill (agent-guided binary + skill version management)
- Enrichment skill (Crustdata, Exa, Partiful integration patterns)
- `gbrain skills doctor`

### Knowledge Gap Pipeline
- `brain_gap` — log what the agent can't answer
- `brain_gaps` — list unresolved gaps
- `brain_gap_approve` — escalate sensitivity (internal → external/redacted, requires audit)
- `gbrain gaps` CLI

### Offline CI Gates (mandatory)
- BEIR subset (NQ + FiQA): nDCG@10, no regression > 2% between releases
- Corpus-reality tests: 7K+ file import, SMS test, temporal sub-chunk test, idempotency, contradiction detection
- Concurrency stress: 4 parallel `brain_put` writers with stale versions → OCC invariants hold
- Embedding migration: model A → model B → rollback, zero cross-model contamination
- Round-trip integrity (semantic + byte-exact)
- Static binary verification (ldd/otool gate)

### Advisory Benchmarks
- LongMemEval R@5 ≥ 85%
- LoCoMo F1 ≥ +30% over FTS5 baseline
- Ragas context_precision + recall

### Release Tooling
- `gbrain validate --all`
- `--json` verified on all commands
- `pipe` mode (JSONL streaming)
- CI/CD release pipeline → GitHub Releases with SHA-256 checksums

## Ship Gate (v0.1.0)

1. All offline CI gates pass
2. Advisory benchmarks documented (not blocking)
3. All 8 skills functional and embedded
4. `gbrain serve` exposes all 20+ MCP tools
5. Cross-compiled binaries on GitHub Releases with SHA-256 checksums
6. `README.md` quick-start works end-to-end

## Reviewer Gates

- **Kif**: benchmark harness review, nDCG@10 baseline, latency p95
- **Scruffy**: unit coverage on gap tracking, alerts, skills doctor
- **Bender**: full concurrency stress suite, kill-before-commit recovery, embedding migration
- **Mom**: pathological corpus (7K+ files, malformed frontmatter, cyclic imports)
- **Nibbler**: sensitivity escalation abuse on knowledge gaps, upgrade skill supply chain
- **Zapp**: launch assets review before GitHub Releases goes public
