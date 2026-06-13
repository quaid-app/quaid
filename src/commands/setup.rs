#![expect(
    clippy::print_stdout,
    reason = "CLI command prints user-facing output to stdout by design"
)]

//! `quaid setup` — onboarding helpers that wire Quaid into MCP clients.
//!
//! [`run`] currently supports `--register-mcp`, which merges a `quaid` entry
//! into each known MCP client config (`~/.claude/mcp.json` and
//! `~/.cursor/mcp.json`) while preserving any servers the user has already
//! configured. Writes are atomic (temp file + rename) and a `.bak` of any
//! pre-existing file is created before replacement. `--dry-run` prints the diff
//! without touching disk.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};

/// A known MCP client config file, relative to `$HOME`.
struct McpClient {
    /// Human-facing label for output.
    label: &'static str,
    /// Path components below `$HOME`, e.g. `[".claude", "mcp.json"]`.
    rel_path: &'static [&'static str],
}

/// MCP client config files `--register-mcp` knows how to merge into.
const MCP_CLIENT_CONFIGS: &[McpClient] = &[
    McpClient {
        label: "Claude Code",
        rel_path: &[".claude", "mcp.json"],
    },
    McpClient {
        label: "Cursor",
        rel_path: &[".cursor", "mcp.json"],
    },
];

/// Outcome of merging the `quaid` server into one client config.
enum MergeOutcome {
    /// The config did not exist; a fresh file would be created.
    Created,
    /// An existing `mcpServers.quaid` entry would be updated.
    Updated,
    /// The existing entry already matches the desired value.
    Unchanged,
}

/// Entry point for `quaid setup`.
///
/// `register_mcp` mirrors the `--register-mcp` flag; when false there is nothing
/// to do yet (future onboarding steps may hang off other flags). `dry_run`
/// prints the planned changes without writing.
pub fn run(register_mcp: bool, dry_run: bool, db_path: &str) -> Result<()> {
    if !register_mcp {
        println!("Nothing to do. Pass --register-mcp to wire Quaid into MCP clients.");
        return Ok(());
    }

    let home = home_dir().context("could not resolve HOME directory")?;
    let resolved_db = resolve_db_display_path(db_path, &home);
    let desired = desired_server_entry(&resolved_db);

    if dry_run {
        println!("Dry run — no files will be written.");
    }
    println!("Resolved DB path: {resolved_db}");

    for client in MCP_CLIENT_CONFIGS {
        let path = client_config_path(&home, client);
        register_one(client, &path, &desired, dry_run)?;
    }

    Ok(())
}

/// Resolves `$HOME` from the environment (`HOME`, then `USERPROFILE` on
/// Windows). We read the env directly rather than going through `dirs` so the
/// behaviour matches the spec's "expand ~ via std env HOME" requirement and so
/// tests can override it with a temp dir.
fn home_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HOME") {
        if !home.is_empty() {
            return Some(PathBuf::from(home));
        }
    }
    std::env::var_os("USERPROFILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Expands a leading `~` in the configured DB path against `home`, leaving
/// absolute and relative paths untouched. The result is what gets written into
/// each MCP config's `env.QUAID_DB`.
fn resolve_db_display_path(db_path: &str, home: &Path) -> String {
    if db_path == "~" {
        return home.display().to_string();
    }
    if let Some(rest) = db_path.strip_prefix("~/") {
        return home.join(rest).display().to_string();
    }
    db_path.to_owned()
}

/// The `mcpServers.quaid` value `--register-mcp` writes.
fn desired_server_entry(db_path: &str) -> Value {
    json!({
        "command": "quaid",
        "args": ["serve"],
        "env": {
            "QUAID_DB": db_path,
        },
    })
}

fn client_config_path(home: &Path, client: &McpClient) -> PathBuf {
    let mut path = home.to_path_buf();
    for component in client.rel_path {
        path.push(component);
    }
    path
}

/// Parses an existing config, merges in `desired`, and (unless `dry_run`) writes
/// it back atomically with a `.bak` of any pre-existing file. Prints exactly
/// what changed.
fn register_one(client: &McpClient, path: &Path, desired: &Value, dry_run: bool) -> Result<()> {
    let display = path.display();

    let mut root = read_config(path)
        .with_context(|| format!("failed to read existing config at {display}"))?;

    let outcome = merge_quaid_server(&mut root, desired)?;

    match outcome {
        MergeOutcome::Unchanged => {
            println!(
                "{} ({display}): already up to date — no changes.",
                client.label
            );
            return Ok(());
        }
        MergeOutcome::Created => {
            println!(
                "{} ({display}): create config with mcpServers.quaid",
                client.label
            );
        }
        MergeOutcome::Updated => {
            println!("{} ({display}): update mcpServers.quaid", client.label);
        }
    }

    let serialized =
        serde_json::to_string_pretty(&root).context("failed to serialize merged MCP config")?;

    if dry_run {
        println!("--- would write {display} ---");
        println!("{serialized}");
        return Ok(());
    }

    write_config_atomically(path, &serialized)
        .with_context(|| format!("failed to write config at {display}"))?;

    if matches!(outcome, MergeOutcome::Updated) {
        println!("  backed up previous config to {display}.bak");
    }
    println!("  wrote {display}");
    Ok(())
}

/// Reads an MCP config file into a JSON object. A missing file yields an empty
/// object so callers can treat create and merge uniformly. A present-but-empty
/// file is also treated as an empty object.
fn read_config(path: &Path) -> Result<Value> {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            if contents.trim().is_empty() {
                return Ok(Value::Object(Map::new()));
            }
            let parsed: Value = serde_json::from_str(&contents)
                .context("existing config is not valid JSON; refusing to overwrite")?;
            if !parsed.is_object() {
                bail!("existing config is not a JSON object; refusing to overwrite");
            }
            Ok(parsed)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Value::Object(Map::new())),
        Err(err) => Err(err.into()),
    }
}

/// Merges `desired` into `root.mcpServers.quaid`, preserving every other key.
/// Returns whether the file would be created, updated, or left unchanged.
fn merge_quaid_server(root: &mut Value, desired: &Value) -> Result<MergeOutcome> {
    let was_empty = root.as_object().is_some_and(Map::is_empty);

    let obj = root
        .as_object_mut()
        .context("config root is not a JSON object")?;

    let servers_entry = obj
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()));
    if !servers_entry.is_object() {
        bail!("existing mcpServers is not a JSON object; refusing to overwrite");
    }
    let servers = servers_entry
        .as_object_mut()
        .context("mcpServers is not a JSON object")?;

    let previously_present = servers.contains_key("quaid");
    if servers.get("quaid") == Some(desired) {
        return Ok(MergeOutcome::Unchanged);
    }

    servers.insert("quaid".to_owned(), desired.clone());

    if previously_present {
        Ok(MergeOutcome::Updated)
    } else if was_empty {
        Ok(MergeOutcome::Created)
    } else {
        // The file existed with other content but no `quaid` server. We are
        // editing in place, so treat it as an update (a .bak is still made).
        Ok(MergeOutcome::Updated)
    }
}

/// Writes `contents` to `path` atomically: write to a sibling temp file, fsync,
/// back up any pre-existing file to `<path>.bak`, then rename into place.
fn write_config_atomically(path: &Path, contents: &str) -> Result<()> {
    use std::io::Write as _;

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }

    if path.exists() {
        let backup = backup_path(path);
        std::fs::copy(path, &backup)
            .with_context(|| format!("failed to back up to {}", backup.display()))?;
    }

    let tmp = temp_path(path);
    {
        let mut file = std::fs::File::create(&tmp)
            .with_context(|| format!("failed to create temp file {}", tmp.display()))?;
        file.write_all(contents.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("failed to rename temp file into {}", path.display()))?;
    Ok(())
}

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".bak");
    PathBuf::from(name)
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".quaid-tmp");
    PathBuf::from(name)
}
