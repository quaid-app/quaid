---
updated_at: 2026-04-15T00:00:00Z
focus_area: Phase 2 — Intelligence Layer (p2-intelligence-layer)
active_issues: [6]
active_branch: phase2/p2-intelligence-layer
---

# What We're Focused On

**Phase 2 is live.** Phase 1 shipped as v0.1.0. The team is now executing the Intelligence Layer on branch `phase2/p2-intelligence-layer`.

**Phase 2 scope:**
- Graph core: N-hop BFS over `links` table with temporal filtering (`src/core/graph.rs`)
- Assertions + contradiction detection (`src/core/assertions.rs`, `src/commands/check.rs`)
- Progressive retrieval with token-budget gating (`src/core/progressive.rs`)
- Novelty check wiring into ingest (`src/commands/ingest.rs`)
- Palace room classification (`src/core/palace.rs::derive_room`)
- Knowledge gaps CLI (`src/core/gaps.rs`, `src/commands/gaps.rs`)
- MCP Phase 2 write surface: `brain_link`, `brain_link_close`, `brain_backlinks`, `brain_graph`, `brain_check`, `brain_timeline`, `brain_tags`

**Team lanes:**
- **Fry**: Groups 1–9 implementation (all core + command + MCP wiring)
- **Scruffy**: 90%+ test coverage — exhaustive unit tests alongside every group
- **Bender**: Integration tests — ingest-conflict → contradiction round-trip, parallel writer OCC
- **Amy**: Update all project docs (README, docs/, spec references)
- **Hermes**: Update website documentation to reflect Phase 2 features
- **Professor**: Gate review — graph BFS correctness, progressive budget logic, OCC protocol
- **Nibbler**: Adversarial review — MCP write surface (link injection, graph depth abuse, contradiction poisoning)
- **Mom**: Temporal edge cases — valid_from/valid_until schema CHECK, zero-hop graph, null valid_from

**Coverage target:** 90%+ (≥200 unit tests, ship gate requires all pass)

**Phase gate requirements:**
- `cargo test` all pass
- `cargo clippy -- -D warnings` clean
- `cargo fmt --check` clean
- Professor + Nibbler sign-off before merge
- Phase 1 round-trip tests (`roundtrip_semantic.rs`, `roundtrip_raw.rs`) show no regression

**Phase sequence:**
- ✅ Sprint 0 → ✅ Phase 1 (v0.1.0 shipped) → 🚀 **Phase 2 (now)** → Phase 3 (Polish + Release)
