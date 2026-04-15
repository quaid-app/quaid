---
id: sprint-0-repo-scaffold
title: "Sprint 0: Repository Scaffold"
status: proposed
type: scaffold
phase: sprint-0
owner: leela
created: 2026-04-13
---

# Sprint 0: Repository Scaffold

## What

Establish the full repository structure, project files, and CI/CD pipelines before any core implementation begins.

## Why

The spec is complete. Before Fry starts Phase 1 implementation, we need:
- A `Cargo.toml` with all declared dependencies so the build graph is visible and reviewable
- Stubbed module tree so contributors know exactly where things go
- `src/schema.sql` with the full v4 schema — this is spec, not implementation
- Skeleton `skills/` files to make the fat-skill architecture tangible
- `tests/` and `benchmarks/` directory scaffolding
- `CLAUDE.md` and `AGENTS.md` so any agent spawned here understands the architecture
- CI and release workflows so PRs are gated from the first commit

## Scope

**In scope:**
- `Cargo.toml` with full dependency declarations (including `clap` `env` feature)
- Module stubs in `src/` (`todo!()` bodies or empty `mod` declarations)
- `src/schema.sql` — full v4 DDL from spec
- `skills/*/SKILL.md` stubs (file + placeholder content for each of 8 skills)
- `tests/fixtures/person.md`, `tests/fixtures/company.md`
- `benchmarks/README.md`
- `CLAUDE.md` and `AGENTS.md`
- `.github/workflows/ci.yml` — `cargo check` + `cargo test` + `cargo fmt` + `cargo clippy` on PRs
- `.github/workflows/release.yml` — cross-compile matrix → GitHub Releases on semver tag push (pinned tooling)

**Out of scope:**
- Any Rust implementation logic
- Full skill content (stubs only — Phase 1 finalizes)
- Benchmark harness logic
- GitHub Releases or version tags

## Non-Goals

- `cargo build --release` doesn't need to succeed yet (stub modules may fail to compile)
- musl/static binary builds are release-only; CI does not require them on PRs
- No model weights embedded yet
- No documentation site

## Success Criteria

1. `cargo check` passes on the stub crate
2. CI workflow triggers on PR and runs `cargo check` + `cargo test`
3. All directories from the spec's Repository Structure section exist
4. `src/schema.sql` matches the v4 DDL in `docs/spec.md`
5. `CLAUDE.md` gives any new agent session enough context to navigate the codebase
