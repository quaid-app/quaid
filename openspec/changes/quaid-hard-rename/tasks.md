# Quaid Hard Rename — Implementation Checklist

**Scope:** Hard rename from the legacy product/binary/tool terminology to Quaid/quaid/memory across the entire repository. No aliases, no shims, no legacy compatibility. Closes the identity migration implied by the repository already living at `quaid-app/quaid`.

**Pre-condition:** `vault-sync-engine` branch must either be merged to `main` first OR its owners (Fry, Professor, Nibbler) must confirm they will rebase onto this rename after it lands. Do not start implementation until that coordination is confirmed.

---

## Phase A — OpenSpec (this change)

- [x] A.1 Create `openspec/changes/quaid-hard-rename/` with `proposal.md`, `tasks.md`, and specs: `binary-rename/spec.md`, `mcp-tool-rename/spec.md`, `schema-migration/spec.md`, `env-var-rename/spec.md`.

---

## Phase B — Schema (breaking; atomic commit required)

Per the breaking-schema-changes skill: DDL, SCHEMA_VERSION bump, and test fixture updates must land in the same commit.

- [x] B.1 Rename the legacy configuration table → `quaid_config` in `src/schema.sql`. Update all column comments that mention the legacy product name.
- [x] B.2 Bump `SCHEMA_VERSION` constant in `src/core/db.rs`.
- [x] B.3 Update all references to the legacy configuration table in `src/core/db.rs` (init, read, write, validate) to `quaid_config`.
- [x] B.4 Update default DB directory in `src/core/db.rs`: legacy dir → `~/.quaid`, default filename `brain.db` → `memory.db`.
- [x] B.5 Update all test fixtures and test helper functions that reference the legacy configuration table, old default paths, or old schema structure.
- [x] B.6 Verify `cargo test` passes in this single commit before proceeding.

---

## Phase C — Cargo / Binary rename

- [x] C.1 Update `Cargo.toml`:
  - `name` → `"quaid"` (was legacy name)
  - `[[bin]] name` → `"quaid"` (was legacy name)
  - `repository` → `https://github.com/quaid-app/quaid`
  - `description` updated to remove legacy product branding
- [x] C.2 Update `src/main.rs`:
  - clap `name` → `"quaid"` (was legacy name)
  - `about` string updated
  - `env = "QUAID_DB"` (previously used legacy env prefix)
  - `env = "QUAID_MODEL"` (previously used legacy env prefix)
  - Any remaining legacy env refs updated to `QUAID_*`
- [x] C.3 Update `build.rs` if it references product name or env vars.

---

## Phase D — MCP tool renames

All 17 tools in `src/mcp/server.rs` renamed. This is a user-visible breaking change for every MCP client.

- [x] D.1 Rename all `#[tool(name = "memory_*")]` annotations (previously used legacy prefix) — see full mapping in `proposal.md`.
- [x] D.2 Rename the Rust method names alongside the tool annotations for consistency (e.g., method `memory_get` for tool `memory_get`).
- [x] D.3 Update any internal references to tool names in error messages, docstrings, or `tool_description!` macro calls.
- [x] D.4 Update `src/mcp/server.rs` `about` / server name string from the legacy product name to Quaid if present.

---

## Phase E — Env var rename

- [x] E.1 Update `scripts/install.sh`:
  - All env vars → `QUAID_*` (previously used legacy prefix)
  - `REPO` → `"quaid-app/quaid"` (previously pointed to legacy repo)
  - Profile injection text (PATH comments, export lines) updated to mention `quaid`
- [x] E.2 Audit all remaining files for legacy env var prefix references (CI workflows, docs, skills):
  `rg "QUAID_" --type-add "text:*.{md,sh,yml,yaml,toml,rs,json,mjs,ts}" -t text` (verify only new-style names appear)
- [x] E.3 Update all found references.

---

## Phase F — CI / Release workflows

- [x] F.1 Update `.github/workflows/release.yml`:
  - Artifact names: `quaid-*` (previously used legacy naming)
  - Binary references
  - Any legacy product name in workflow titles/display names
- [x] F.2 Update `.github/workflows/ci.yml`:
  - Any legacy binary calls → `quaid`
  - Cache key names if they include the legacy product name
- [x] F.3 Update `.github/workflows/publish-npm.yml` if it references the product name.
- [x] F.4 Update `.github/RELEASE_CHECKLIST.md`.

---

## Phase G — Documentation

- [x] G.1 `README.md` — comprehensive pass: title, tagline, all `quaid` CLI examples, all `QUAID_*` env var examples, all `memory_*` MCP tool examples, repo URL, install instructions.
- [x] G.2 `CLAUDE.md` — update product name, binary name, env vars, MCP tool names, default DB path.
- [x] G.3 `docs/spec.md` — update title, all CLI references, MCP tool table, env var table, default paths. Update `status` frontmatter field if it names the product.
- [x] G.4 `docs/getting-started.md` — all quickstart commands.
- [x] G.5 `docs/contributing.md` — all tool references.
- [x] G.6 `docs/roadmap.md` — product name, any CLI examples.
- [x] G.7 `docs/` friction analysis doc — update any remaining product-name references (or rename file if appropriate — consult with macro88 before renaming files in `docs/`).
- [x] G.8 `website/` — update `package.json` (description, name if legacy product name appears), `DESIGN.md`, `SITE.md`, any Astro content pages, `astro.config.mjs` site title/description.

---

## Phase H — Skills

All skill SKILL.md files previously used the legacy CLI binary name and legacy MCP tool prefix.

- [x] H.1 `skills/ingest/SKILL.md` — all CLI examples → `quaid`, MCP tools → `memory_*`, env vars → `QUAID_*`.
- [x] H.2 `skills/query/SKILL.md` — same.
- [x] H.3 `skills/maintain/SKILL.md` — same.
- [x] H.4 `skills/briefing/SKILL.md` — same.
- [x] H.5 `skills/research/SKILL.md` — same.
- [x] H.6 All remaining `skills/*/SKILL.md` files — same.
- [x] H.7 Run a search across `skills/` for any remaining occurrences of the legacy CLI name, legacy tool prefix, or legacy env prefix to confirm zero remaining occurrences.

---

## Phase I — Test suite

- [x] I.1 Audit all test files for hard-coded legacy binary name, legacy MCP tool prefix, legacy configuration table name, legacy env var prefix, or old default path strings.
  Run a search across `tests/` and `src/` (Rust files) for these legacy patterns.
- [x] I.2 Update all found occurrences.
- [x] I.3 Update any integration tests that invoke the binary by name (legacy name → `./quaid` or `cargo run --bin quaid`).
- [x] I.4 `cargo test` must pass with zero failures.

---

## Phase J — Final audit

- [ ] J.1 Run exhaustive repo-wide scan for any remaining legacy product-name or legacy tool/env/table references across all text-type files.
  Expected result: zero files (excluding `.squad/` agent history files, which are historical record and explicitly excluded from this rename).
- [x] J.2 Confirm `openspec/` existing change directories have had their prose updated (G.x not this task, but verify).
- [x] J.3 Confirm `Cargo.lock` is regenerated after `Cargo.toml` rename (`cargo build` touch).
- [x] J.4 `cargo build --release` succeeds and produces a binary named `quaid`.

---

## Phase K — Migration guide

- [ ] K.1 Add a `MIGRATION.md` (or a section in `README.md`) documenting:
  - The old-to-new binary name change
  - The old-to-new env var table
  - The old-to-new MCP tool name table
  - The DB migration path: `quaid export > backup.tar && quaid init ~/.quaid/memory.db && quaid import backup/` (using the old binary version for export, then `quaid` for init/import)
  - That MCP client configs (Claude Code, etc.) must be manually updated

---

## Phase L — PR

- [x] L.1 `cargo test` green.
- [x] L.2 `cargo build --release` produces `quaid` binary.
- [ ] L.3 Open PR against `main`. Title: `chore: hard rename legacy product → Quaid, quaid, memory_*, QUAID_*`. Body references this openspec change directory.

---

## Ownership recommendations

| Phase | Recommended owner | Reviewer(s) |
|-------|------------------|-------------|
| B — Schema | Professor (schema invariants) | Nibbler (test coverage), Leela (gate) |
| C — Cargo/binary | Fry | Professor |
| D — MCP tools | Fry or Hermes | Professor |
| E — Env vars | Fry | Leela |
| F — CI/Release | Zapp or Kif | Leela |
| G — Docs | Scribe or Amy | Leela |
| H — Skills | Amy or Scribe | Professor |
| I — Tests | Scruffy | Professor, Nibbler |
| J — Audit | Nibbler | Leela |
| K — Migration guide | Amy or Scribe | Leela |
