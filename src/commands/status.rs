#![expect(
    clippy::print_stdout,
    reason = "CLI command prints user-facing output to stdout by design"
)]

//! `quaid status` — process-level status.
//!
//! Reports daemon installed/running state, the active runtime-host
//! `session_type` (`daemon` / `serve_host` / none), database path,
//! schema version, last extraction-queue activity, and the last
//! vault-sync heartbeat. Distinct from `quaid stats` (content-level
//! statistics like page count and index health).

use anyhow::Result;
use rusqlite::Connection;
use serde_json::json;

use crate::core::vault_sync;

/// Run the status command and return the exit code to use.
///
/// Exit codes mirror `quaid daemon status` so cron-style automation
/// can rely on them: `0` daemon running, `1` installed-but-stopped,
/// `2` not installed, `3` unexpected error (e.g. db read failure).
pub fn run(db: &Connection, json_output: bool) -> Result<u8> {
    let db_path = vault_sync::database_path(db).unwrap_or_else(|_| "<unknown>".to_string());

    let runtime_host = vault_sync::find_active_runtime_host(db).ok().flatten();

    let daemon_unit = daemon_unit_status();
    let last_extraction = read_last_extraction_at(db).ok().flatten();
    let last_heartbeat = read_last_heartbeat_at(db).ok().flatten();

    let exit_code = match daemon_unit {
        UnitState::Running => 0u8,
        UnitState::InstalledStopped => 1,
        UnitState::NotInstalled => 2,
        UnitState::ProbeFailed => 3, // service-manager probe errored — unknown state
        UnitState::Unsupported => 0, // not an error: just no platform integration
    };

    if json_output {
        let payload = json!({
            "daemon": {
                "installed": matches!(daemon_unit, UnitState::Running | UnitState::InstalledStopped),
                "running": matches!(daemon_unit, UnitState::Running),
                "platform_supported": !matches!(daemon_unit, UnitState::Unsupported),
                "probe_failed": matches!(daemon_unit, UnitState::ProbeFailed),
            },
            "runtime_host": runtime_host.as_ref().map(|h| json!({
                "session_type": h.session_type,
                "session_id": h.session_id,
                "pid": h.pid,
                "host": h.host,
            })),
            "transports": {
                "stdio": "available",
                "http": "opt-in via `quaid serve --http` or `quaid daemon install --http`",
            },
            "database": {
                "path": db_path,
            },
            "activity": {
                "last_extraction_at": last_extraction,
                "last_heartbeat_at": last_heartbeat,
            },
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("daemon:");
        match daemon_unit {
            UnitState::Running => println!("  state: running"),
            UnitState::InstalledStopped => println!("  state: installed; stopped"),
            UnitState::NotInstalled => {
                println!("  state: not installed (run `quaid daemon install` to set up)")
            }
            UnitState::ProbeFailed => {
                println!("  state: unknown (service-manager status probe failed)")
            }
            UnitState::Unsupported => {
                println!("  state: platform not supported (macOS and Linux only)")
            }
        }

        println!("runtime_host:");
        match &runtime_host {
            Some(host) => {
                println!("  session_type: {}", host.session_type);
                println!("  pid: {}", host.pid);
                println!("  host: {}", host.host);
                println!("  session_id: {}", host.session_id);
            }
            None => println!("  none (no live daemon or serve_host)"),
        }

        println!("database:");
        println!("  path: {db_path}");

        println!("activity:");
        match last_extraction {
            Some(ts) => println!("  last_extraction_at: {ts}"),
            None => println!("  last_extraction_at: (none)"),
        }
        match last_heartbeat {
            Some(ts) => println!("  last_heartbeat_at: {ts}"),
            None => println!("  last_heartbeat_at: (none)"),
        }

        println!("transports:");
        println!("  stdio: always available via `quaid serve`");
        println!("  http: opt-in via `quaid serve --http` or `quaid daemon install --http`");
    }

    Ok(exit_code)
}

#[derive(Debug, Clone, Copy)]
#[allow(
    dead_code,
    reason = "Unsupported and ProbeFailed variants are only constructed on a subset of targets; the cfg-gated daemon_unit_status uses the same enum across all targets so some variants are unreachable per-platform but still part of the type"
)]
enum UnitState {
    Running,
    InstalledStopped,
    NotInstalled,
    /// The service-manager probe itself errored (e.g. `launchctl` /
    /// `systemctl` could not be executed) — distinct from a clean
    /// "not installed" answer.
    ProbeFailed,
    Unsupported,
}

#[cfg(target_os = "macos")]
fn daemon_unit_status() -> UnitState {
    match crate::platform::launchd::status() {
        Ok(crate::platform::UnitStatus::Running) => UnitState::Running,
        Ok(crate::platform::UnitStatus::InstalledStopped) => UnitState::InstalledStopped,
        Ok(crate::platform::UnitStatus::NotInstalled) => UnitState::NotInstalled,
        Err(_) => UnitState::ProbeFailed,
    }
}

#[cfg(target_os = "linux")]
fn daemon_unit_status() -> UnitState {
    match crate::platform::systemd::status() {
        Ok(crate::platform::UnitStatus::Running) => UnitState::Running,
        Ok(crate::platform::UnitStatus::InstalledStopped) => UnitState::InstalledStopped,
        Ok(crate::platform::UnitStatus::NotInstalled) => UnitState::NotInstalled,
        Err(_) => UnitState::ProbeFailed,
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn daemon_unit_status() -> UnitState {
    UnitState::Unsupported
}

fn read_last_extraction_at(db: &Connection) -> Result<Option<String>> {
    let row: Option<String> = db
        .query_row(
            "SELECT MAX(CASE
                        WHEN scheduled_for > enqueued_at THEN scheduled_for
                        ELSE enqueued_at
                    END)
             FROM extraction_queue",
            [],
            |row| row.get(0),
        )
        .ok()
        .flatten();
    Ok(row)
}

fn read_last_heartbeat_at(db: &Connection) -> Result<Option<String>> {
    let row: Option<String> = db
        .query_row(
            "SELECT MAX(heartbeat_at) FROM serve_sessions
             WHERE session_type IN ('daemon', 'serve_host')",
            [],
            |row| row.get(0),
        )
        .ok()
        .flatten();
    Ok(row)
}
