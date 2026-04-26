# Quaid Hard Rename — Implementation Checklist

**Scope:** Hard rename from GigaBrain/gbrain/brain terminology to Quaid/quaid/memory across the entire repository. No aliases, no shims, no legacy compatibility. Closes the identity migration implied by the repository already living at `quaid-app/quaid`.

**Pre-condition:** `vault-sync-engine` branch must either be merged to `main` first OR its owners (Fry, Professor, Nibbler) must confirm they will rebase onto this rename after it lands. Do not start implementation until that coordination is confirmed.

---

## Phase A — OpenSpec (this change)

- [x] A.1 Create `openspec/changes/quaid-hard-rename/` with `proposal.md`, `tasks.md`, and specs: `binary-rename/spec.md`, `mcp-tool-rename/spec.md`, `schema-migration/spec.md`, `env-var-rename/spec.md`.

---

## Phase B — Schema (breaking; atomic commit required)

Per the breaking-schema-changes skill: DDL, SCHEMA_VERSION bump, and test fixture updates must land in the same commit.

- [ ] B.1 Rename `brain_config` → `quaid_config` in `src/schema.sql`. Update all column comments that mention GigaBrain.
- [ ] B.2 Bump `SCHEMA_VERSION` constant in `src/core/db.rs`.
- [ ] B.3 Update all references to `brain_config` in `src/core/db.rs` (init, read, write, validate) to `quaid_config`.
- [ ] B.4 Update default DB directory in `src/core/db.rs`: `~/.gbrain` → `~/.quaid`, default filename `brain.db` → `memory.db`.
- [ ] B.5 Update all test fixtures and test helper functions that reference `brain_config`, old default paths, or old schema structure.
- [ ] B.6 Verify `cargo test` passes in this single commit before proceeding.

---

## Phase C — Cargo / Binary rename

- [ ] C.1 Update `Cargo.toml`:
  - `name = "gbrain"` → `name = "quaid"`
  - `[[bin]] name = "gbrain"` → `[[bin]] name = "quaid"`
  - `repository` → `https://github.com/quaid-app/quaid`
  - `description` updated to remove GigaBrain branding
- [ ] C.2 Update `src/main.rs`:
  - clap `name = "gbrain"` → `name = "quaid"`
  - `about` string updated
  - `env = "GBRAIN_DB"` → `env = "QUAID_DB"`
  - `env = "GBRAIN_MODEL"` → `env = "QUAID_MODEL"`
  - Any remaining `GBRAIN_*` env refs updated to `QUAID_*`
- [ ] C.3 Update `build.rs` if it references product name or env vars.

---

## Phase D — MCP tool renames

All 17 tools in `src/mcp/server.rs` renamed. This is a user-visible breaking change for every MCP client.

- [ ] D.1 Rename all `#[tool(name = "brain_*")]` annotations to `memory_*` (see full mapping in `proposal.md`).
- [ ] D.2 Rename the Rust method names alongside the tool annotations for consistency (e.g., `brain_get` → `memory_get`).
- [ ] D.3 Update any internal references to tool names in error messages, docstrings, or `tool_description!` macro calls.
- [ ] D.4 Update `src/mcp/server.rs` `about` / server name string from GigaBrain to Quaid if present.

---

## Phase E — Env var rename

- [ ] E.1 Update `scripts/install.sh`:
  - All `GBRAIN_*` variables → `QUAID_*`
  - `REPO="macro88/gigabrain"` → `REPO="quaid-app/quaid"`
  - Profile injection text (PATH comments, export lines) updated to mention `quaid`
- [ ] E.2 Audit all remaining files for `GBRAIN_*` references (CI workflows, docs, skills):
  `rg "GBRAIN_" --type-add "text:*.{md,sh,yml,yaml,toml,rs,json,mjs,ts}" -t text`
- [ ] E.3 Update all found references.

---

## Phase F — CI / Release workflows

- [ ] F.1 Update `.github/workflows/release.yml`:
  - Artifact names: `gbrain-*` → `quaid-*`
  - Binary references
  - Any GigaBrain in workflow titles/display names
- [ ] F.2 Update `.github/workflows/ci.yml`:
  - Any `gbrain` binary calls → `quaid`
  - Cache key names if they include product name
- [ ] F.3 Update `.github/workflows/publish-npm.yml` if it references the product name.
- [ ] F.4 Update `.github/RELEASE_CHECKLIST.md`.

---

## Phase G — Documentation

- [ ] G.1 `README.md` — comprehensive pass: title, tagline, all `gbrain` CLI examples, all `GBRAIN_*` env var examples, all `brain_*` MCP tool examples, repo URL, install instructions.
- [ ] G.2 `CLAUDE.md` — update product name, binary name, env vars, MCP tool names, default DB path.
- [ ] G.3 `docs/spec.md` — update title, all CLI references, MCP tool table, env var table, default paths. Update `status` frontmatter field if it names the product.
- [ ] G.4 `docs/getting-started.md` — all quickstart commands.
- [ ] G.5 `docs/contributing.md` — all tool references.
- [ ] G.6 `docs/roadmap.md` — product name, any CLI examples.
- [ ] G.7 `docs/gigabrain-vs-qmd-friction-analysis.md` — update product-name references (or rename file if appropriate — consult with macro88 before renaming files in `docs/`).
- [x] G.8 `website/` — update `package.json` (description, name if `gigabrain` appears), `DESIGN.md`, `SITE.md`, any Astro content pages, `astro.config.mjs` site title/description.

---

## Phase H — Skills

All skill SKILL.md files use `gbrain` CLI examples and `brain_*` MCP tool calls.

- [ ] H.1 `skills/ingest/SKILL.md` — all `gbrain` → `quaid`, `brain_*` → `memory_*`, `GBRAIN_*` → `QUAID_*`.
- [ ] H.2 `skills/query/SKILL.md` — same.
- [ ] H.3 `skills/maintain/SKILL.md` — same.
- [ ] H.4 `skills/briefing/SKILL.md` — same.
- [ ] H.5 `skills/research/SKILL.md` — same.
- [ ] H.6 All remaining `skills/*/SKILL.md` files — same.
- [ ] H.7 Run: `rg "gbrain|GigaBrain|brain_|GBRAIN_" skills/ --type md` to confirm zero remaining occurrences.

---

## Phase I — Test suite

- [ ] I.1 Audit all test files for hard-coded `gbrain`, `brain_*` tool names, `brain_config` table names, `GBRAIN_*` env vars, old default path strings.
  `rg "gbrain|brain_config|GBRAIN_" tests/ src/ --type rust`
- [ ] I.2 Update all found occurrences.
- [ ] I.3 Update any integration tests that invoke the binary by name (`./gbrain` → `./quaid` or `cargo run --bin quaid`).
- [ ] I.4 `cargo test` must pass with zero failures.

---

## Phase J — Final audit

- [ ] J.1 Run exhaustive repo-wide scan:
  `rg -i "gbrain|gigabrain|brain\.db|brain_config|GBRAIN_" --type-add "text:*.{rs,md,toml,sh,yml,yaml,json,mjs,ts}" -t text -l`
  Expected result: zero files (excluding `.squad/` agent history files, which are historical record and explicitly excluded from this rename).
- [ ] J.2 Confirm `openspec/` existing change directories have had their prose updated (G.x not this task, but verify).
- [ ] J.3 Confirm `Cargo.lock` is regenerated after `Cargo.toml` rename (`cargo build` touch).
- [ ] J.4 `cargo build --release` succeeds and produces a binary named `quaid`.

---

## Phase K — Migration guide

- [ ] K.1 Add a `MIGRATION.md` (or a section in `README.md`) documenting:
  - The old-to-new binary name change
  - The old-to-new env var table
  - The old-to-new MCP tool name table
  - The DB migration path: `gbrain export > backup.tar && quaid init ~/.quaid/memory.db && quaid import backup/`
  - That MCP client configs (Claude Code, etc.) must be manually updated

---

## Phase L — PR

- [ ] L.1 `cargo test` green.
- [ ] L.2 `cargo build --release` produces `quaid` binary.
- [ ] L.3 Open PR against `main`. Title: `chore: hard rename GigaBrain→Quaid, gbrain→quaid, brain_*→memory_*, GBRAIN_*→QUAID_*`. Body references this openspec change directory.

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
