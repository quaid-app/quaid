# Migration Guide — Hard Rename to Quaid

> ⚠️ **Hard break. No supported in-place upgrade path exists from any pre-rename release.**
>
> Every user-facing surface changed at once: the binary name, all environment variables, all
> MCP tool names, the default database path, and the internal schema configuration table.
> There are no shims, no aliases, and no automatic migration code.
>
> If you are coming from a pre-rename installation you are not performing a routine upgrade —
> you are replacing one tool with a different one. **Start fresh with `quaid`.** If you need
> to recover data from a pre-rename database, see [Data recovery](#data-recovery) below.
> This repository does not carry a migration tool, and pre-rename databases cannot be opened
> with `quaid`.

---

## What Quaid looks like now

The complete new surface. Pre-rename names have no supported presence in this repository.

| Surface | Value |
|---------|-------|
| Binary | `quaid` |
| Default database path | `~/.quaid/memory.db` |
| DB path env var | `QUAID_DB` |
| Model selection env var | `QUAID_MODEL` |
| Installer channel env var | `QUAID_CHANNEL` |
| Install directory env var | `QUAID_INSTALL_DIR` |
| Version pin env var | `QUAID_VERSION` |
| No-profile flag env var | `QUAID_NO_PROFILE` |
| GitHub repository | `quaid-app/quaid` |

### MCP tools (all 17)

| Tool |
|------|
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

---

## Install Quaid

```bash
curl -fsSL https://raw.githubusercontent.com/quaid-app/quaid/main/scripts/install.sh | sh
```

Or build from source:

```bash
git clone https://github.com/quaid-app/quaid
cd quaid
cargo build --release
```

See [`docs/getting-started.md`](docs/getting-started.md) for full install options including
GitHub Releases binaries, channel selection, and cross-compilation.

---

## Shell profile

Remove any old env var exports from your shell profile (`~/.zshrc`, `~/.bashrc`, etc.) and
add the `QUAID_*` equivalents:

```bash
export QUAID_DB=~/.quaid/memory.db
```

The shell installer writes this automatically. If you installed manually, add it yourself.

---

## MCP client config

Every MCP client config (Claude Code `.mcp.json`, Cursor, or any other client) that
referenced the pre-rename binary or tool prefix must be rebuilt from scratch. Correct entry:

```json
{
  "mcpServers": {
    "quaid": {
      "command": "quaid",
      "args": ["serve"],
      "env": { "QUAID_DB": "/path/to/memory.db" }
    }
  }
}
```

Clients will silently expose zero tools until the config points to `quaid serve` and all
tool names match the `memory_*` prefix.

---

## Data recovery

Databases created with the pre-rename binary are **schema-incompatible** with `quaid`.
The internal configuration table was renamed and `SCHEMA_VERSION` was bumped to `6`.
`quaid` will refuse to open a pre-rename database.

**This repository does not provide a migration tool.** If you need to recover your pages:

1. Locate your pre-rename binary. It is not in this repository — check your local `PATH`,
   your system package manager, or the GitHub Releases page of the pre-rename project.
2. Use that binary to export your pages to a local directory. Consult the documentation for
   that release — it is outside the scope of this guide.
3. Initialize a new Quaid database:
   ```bash
   quaid init ~/.quaid/memory.db
   ```
4. Import the exported pages:
   ```bash
   quaid import <backup-directory>/ --db ~/.quaid/memory.db
   ```

> **What survives the round-trip:** page content, links, tags, and timeline entries.
> Embedding vectors are recomputed on first query. Knowledge gaps and raw import history
> are not carried over.

If you cannot locate the pre-rename binary you cannot open a pre-rename database with
`quaid`. There is no recovery path in this repository.

---

## npm

If you previously installed the pre-rename package via npm, uninstall it first:

```bash
# Identify what is installed globally
npm list -g --depth 0

# Uninstall the pre-rename package
npm uninstall -g <pre-rename-package-name>
```

`quaid` is staged but **not yet in the public registry** — `npm install -g quaid` will not work
yet. After uninstalling the old package, install using the shell installer or a GitHub Releases
binary until the npm channel is live.
