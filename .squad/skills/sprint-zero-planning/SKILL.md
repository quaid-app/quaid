---
name: "sprint-zero-planning"
description: "How to run Sprint 0 for a new Rust project in this squad"
domain: "project-setup"
confidence: "high"
source: "earned"
---

## Context

Sprint 0 runs before any implementation begins. Its job is to establish structure,
gates, and ownership so all parallel workstreams (impl, test, docs, review) stay
coordinated from the first commit.

## Patterns

1. **Read the spec first** — understand phases, ship gates, and reviewer requirements before decomposing.

2. **Create OpenSpec proposals for each major phase** before touching code. Proposals live in `openspec/changes/<name>/proposal.md`. They must include: scope, non-goals, ship gates, and reviewer assignments.

3. **Scaffold structure matches the spec's Repository Structure section** exactly. Stubs use `todo!()` — no logic, just the right signatures and module tree.

4. **Schema is spec, not implementation** — `src/schema.sql` should be created in Sprint 0 from the DDL in the spec. It's the contract everything else builds to.

5. **CI gates from day one** — `ci.yml` should run on every PR: `cargo fmt`, `cargo clippy`, `cargo check`, `cargo test`, static binary verification. Don't wait for Phase 3 to add these.

6. **Phase gates must be explicit** — each OpenSpec proposal's `depends_on` and `Ship Gate` section enforce sequencing. No Phase 2 proposal should be actionable until Phase 1 gates pass.

7. **CLAUDE.md and AGENTS.md belong in Sprint 0** — any agent spawned in the repo needs to orient quickly. These files should be created before implementation starts.

## Examples

See `openspec/changes/sprint-0-repo-scaffold/proposal.md` for a complete Sprint 0 proposal.

## Anti-Patterns

- Do NOT implement Phase 2+ features in Sprint 0. Scope creep kills scaffolds.
- Do NOT skip `src/schema.sql` — without it, `db.rs` has no contract to implement against.
- Do NOT let Sprint 0 end without CI triggering on a PR. "We'll add CI later" means never.
- Do NOT use the `create` tool to create directories — it requires parent dirs to exist. Use a general-purpose agent with Python to create directory trees first.

## Environment Notes (Windows, no pwsh.exe)

- `pwsh.exe` (PowerShell 7) is NOT available. Use a general-purpose agent with Python to create directory trees.
- GitHub write tools (create issue, create PR) are not available via MCP. Document required git/GitHub actions for the user.
