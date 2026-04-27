## Why

The project has grown into a production-quality tool but shipped under a prior name that was descriptive-but-generic and carried heritage branding from its inspiration. The canonical repository is already at `quaid-app/quaid`. The rename completes the identity migration: product is **Quaid**, CLI binary is `quaid`, the conceptual layer is **memory**, and all MCP tools become `memory_*`. This is a hard rename — no legacy aliases, no shims, no compatibility wrappers.

## What Changes

### Surface-level (user-visible)

| Before | After |
|--------|-------|
| Binary name | *(legacy)* | `quaid` |
| Product name | *(legacy)* | Quaid |
| Crate name (`Cargo.toml`) | *(legacy)* | `quaid` |
| Default DB filename | `brain.db` | `memory.db` |
| Default DB directory | `~/.quaid/` (was legacy dir) | `~/.quaid/` |
| Default DB full path | *(legacy path)* | `~/.quaid/memory.db` |
| Env var: DB path | *(legacy)* | `QUAID_DB` |
| Env var: model selection | *(legacy)* | `QUAID_MODEL` |
| Env var: channel | *(legacy)* | `QUAID_CHANNEL` |
| Env var: install dir | *(legacy)* | `QUAID_INSTALL_DIR` |
| Env var: version | *(legacy)* | `QUAID_VERSION` |
| Env var: no-profile | *(legacy)* | `QUAID_NO_PROFILE` |
| Env var: release API URL | *(legacy)* | `QUAID_RELEASE_API_URL` |
| Env var: release base URL | *(legacy)* | `QUAID_RELEASE_BASE_URL` |
| GitHub repo slug | *(legacy)* | `quaid-app/quaid` |
| MCP tool prefix | *(legacy prefix)* | `memory_*` |

### MCP tool renames (exhaustive)

All 17 tools migrated from the legacy prefix to the `memory_*` prefix:

| New name |
|----------|
| `memory_get` |
| `memory_put` |
| `memory_query` |
| `memory_search` |
| `memory_list` |
| `memory_link` |
| `memory_link_close` |
| `memory_backlinks` |
| `memory_graph` |
| `memory_timeline` |
| `memory_tags` |
| `memory_check` |
| `memory_gap` |
| `memory_gaps` |
| `memory_stats` |
| `memory_raw` |
| `memory_collections` |

### Schema changes

The internal table `quaid_config` (previously named with the legacy product prefix; stores model metadata) replaces the old configuration table. All other table names (`pages`, `page_fts`, `page_embeddings`, `page_embeddings_vec_*`, `links`, `assertions`, `knowledge_gaps`, `ingest_log`) are unchanged — they name domain concepts, not the product.

This constitutes a **breaking schema change** requiring a `SCHEMA_VERSION` bump. Databases created with the legacy binary are incompatible with `quaid`. Migration path: export with the previous binary version, re-init with `quaid init`, re-import. No automatic migration is provided (consistent with "no legacy support" directive).

### Code-level

- `Cargo.toml`: `name = "quaid"`, `[[bin]] name = "quaid"`, `repository` updated to `quaid-app/quaid`
- `src/main.rs`: clap `name = "quaid"`, env var references updated
- `src/mcp/server.rs`: all `#[tool(name = "memory_*")]` annotations and method names in place (previously used a legacy prefix)
- `src/core/db.rs`: `quaid_config` table, `SCHEMA_VERSION` bumped, `~/.quaid` default path (previously pointed to legacy directory)
- `src/schema.sql`: `quaid_config` DDL in place; any remaining legacy product-name comments updated
- `build.rs`: any legacy product-name references updated
- `scripts/install.sh`: all `QUAID_*` env vars, `quaid-app/quaid` repo slug, profile-injection text (previously used legacy names)
- `README.md`, `CLAUDE.md`, `docs/spec.md`, `docs/getting-started.md`, `docs/contributing.md`, `docs/roadmap.md`: all product-name occurrences updated
- `website/`: all site content, metadata, package.json (if legacy product name appeared)
- `.github/workflows/`: release artifact names (`quaid-*`), workflow titles (previously used legacy naming)
- `skills/*/SKILL.md`: all CLI examples, env var examples, MCP tool examples use new names (previously used legacy names)
- `openspec/`: existing change proposals updated in their prose to reflect new names (non-structural)
- `tests/`: test fixture strings, DB path helpers, MCP tool name assertions

## Capabilities

### Modified Capabilities
- `cli-binary`: Binary name is now `quaid`. Shell completions, PATH entries, MCP config blocks all need updating by users (legacy binary name no longer provided).
- `mcp-server`: All 17 tool names now use the `memory_*` prefix. Any MCP client config (Claude Code `.mcp.json`, etc.) must be updated.
- `default-db-path`: Default DB location is now `~/.quaid/memory.db`. Explicit `QUAID_DB` env or `--db` flag still overrides.
- `env-vars`: All environment variables now use the `QUAID_*` prefix (previously used a legacy prefix).

### Removed Capabilities
- No legacy binary alias, symlink, or wrapper is provided.
- No legacy MCP tool aliases are provided.
- No legacy-to-new env var forwarding shim.
- No automatic migration of the legacy configuration table to `quaid_config` in existing DBs.

## Non-Goals

- Re-architecture of any runtime behaviour (search, embeddings, storage model, skill system).
- Version bump of any user-facing functionality beyond the rename.
- Providing a migration tool from old to new DB format (manual export/re-init is the documented path).
- Renaming internal non-surface tables (`pages`, `links`, `assertions`, etc.).
- Updating `.squad/` agent history files (historical record; should not be retroactively rewritten).

## Impact

- **230 files** contained at least one legacy product name or legacy tool/variable reference (scanned 2026-04-25).
- **Breaking schema change** (legacy configuration table → `quaid_config`) requires `SCHEMA_VERSION` bump. Existing databases cannot be opened without re-init.
- **Active branch conflict**: `vault-sync-engine` is in-flight on `spec/vault-sync-engine` and touches many of the same files. This rename should land on `main` first; vault-sync owners (Fry, Professor, Nibbler) must rebase. Coordinate before any rename PR is raised.
- **MCP client configs**: Any live Claude Code / other MCP client `mcp.json` referencing the legacy binary path or legacy tool names will break silently until updated.
- **CI release artifacts**: Release workflow previously produced artifacts using the legacy name pattern. These now follow the `quaid-*` naming. Any downstream install scripts pinned to the old asset naming pattern will break.

## Risks

1. **vault-sync rebase complexity** — the active branch is large; a conflict-heavy rebase post-rename could introduce bugs. Mitigation: freeze vault-sync slice selection until rename is merged, or merge vault-sync first.
2. **External MCP client configs** — users with live `~/.config/claude/claude_desktop_config.json` or similar will see all tools disappear until they update the binary command and all tool names to the new values. Mitigation: prominent migration note in README and release notes.
3. **Schema version mismatch on existing DBs** — users upgrading from any previous binary version will be unable to open their database with `quaid`. Mitigation: document the export → `quaid init` → `quaid import` path in the migration guide before the release tag is published.
