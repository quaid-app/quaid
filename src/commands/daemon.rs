#![expect(
    clippy::print_stdout,
    reason = "CLI command prints user-facing output to stdout by design"
)]

//! `quaid daemon` subcommand surface.
//!
//! Backs the `Run | Install | Uninstall | Start | Stop | Restart |
//! Status | Logs` actions defined in `src/main.rs`. The `Run` variant
//! is the foreground entry point that launchd/systemd execute; the
//! other variants drive the platform service manager via the
//! [`crate::platform`] helpers.

use std::net::IpAddr;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::Subcommand;
use rusqlite::Connection;

#[cfg(unix)]
use crate::commands::shutdown::ShutdownSignal;
use crate::core::vault_sync;
use crate::mcp::http::{DEFAULT_HTTP_BIND, DEFAULT_HTTP_PORT};
use crate::mcp::HttpConfig;

/// Sub-actions for the `quaid daemon` command group.
#[derive(Debug, Clone, Subcommand)]
pub enum DaemonAction {
    /// Run the daemon in the foreground. Invoked by launchd/systemd
    /// after `quaid daemon install`.
    Run {
        /// Open the HTTP/SSE MCP transport in addition to the
        /// background runtime.
        #[arg(long)]
        http: bool,
        /// TCP port for the HTTP transport (default: 3112). Requires `--http`.
        #[arg(long, requires = "http")]
        port: Option<u16>,
        /// Bind address (default: 127.0.0.1). Requires `--http`.
        #[arg(long, requires = "http")]
        bind: Option<IpAddr>,
        /// Path to a bearer-token file (parsed but not enforced in v1).
        #[arg(long, requires = "http")]
        token_file: Option<PathBuf>,
        /// Treat loopback as trusted (unauthenticated). Requires `--http`.
        #[arg(long, requires = "http")]
        trust_loopback: bool,
    },
    /// Install a platform-native service unit pointing at the active
    /// database, then start it.
    Install {
        /// Open the HTTP/SSE MCP transport in the installed daemon.
        #[arg(long)]
        http: bool,
        /// TCP port for the HTTP transport. Requires `--http`.
        #[arg(long, requires = "http")]
        port: Option<u16>,
        /// Bind address. Requires `--http`.
        #[arg(long, requires = "http")]
        bind: Option<IpAddr>,
        /// Path to a bearer-token file. Requires `--http`.
        #[arg(long, requires = "http")]
        token_file: Option<PathBuf>,
        /// Treat loopback as trusted. Requires `--http`.
        #[arg(long, requires = "http")]
        trust_loopback: bool,
    },
    /// Stop and remove the installed service unit. Idempotent.
    Uninstall,
    /// Start the installed unit via the platform service manager.
    Start,
    /// Stop the installed unit via the platform service manager.
    Stop,
    /// Restart the installed unit (`stop` + `start`).
    Restart,
    /// Report whether the daemon is installed/running, its PID, the
    /// database path, last extraction activity, and HTTP transport
    /// state when configured.
    Status {
        /// Emit machine-readable JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },
    /// Tail the daemon's stdout/stderr — `tail -F` on the launchd
    /// `StandardErrorPath` (macOS) or `journalctl --user -u quaid-daemon` (Linux).
    Logs {
        /// Stream new lines as they arrive (`tail -F` / `journalctl -f`).
        #[arg(long)]
        follow: bool,
        /// On macOS, also include the stdout log. No-op on Linux
        /// (journald merges both streams).
        #[arg(long)]
        all_streams: bool,
    },
}

/// Top-level dispatch. Each arm is intentionally small and delegates
/// to a focused helper so error mapping can be uniform.
pub async fn run(action: DaemonAction, db: Connection) -> Result<u8> {
    match action {
        DaemonAction::Run {
            http,
            port,
            bind,
            token_file,
            trust_loopback,
        } => {
            let http_config = if http {
                Some(HttpConfig {
                    port: port.unwrap_or(DEFAULT_HTTP_PORT),
                    bind: bind.unwrap_or(DEFAULT_HTTP_BIND),
                    token_file,
                    trusted_loopback: trust_loopback,
                })
            } else {
                None
            };
            run_foreground(db, http_config).await
        }
        DaemonAction::Install {
            http,
            port,
            bind,
            token_file,
            trust_loopback,
        } => install_action(db, http, port, bind, token_file, trust_loopback),
        DaemonAction::Uninstall => uninstall_action(),
        DaemonAction::Start => start_action(),
        DaemonAction::Stop => stop_action(),
        DaemonAction::Restart => restart_action(),
        DaemonAction::Status { json } => status_action(db, json),
        DaemonAction::Logs {
            follow,
            all_streams,
        } => logs_action(follow, all_streams),
    }
}

async fn run_foreground(db: Connection, http_config: Option<HttpConfig>) -> Result<u8> {
    let db_path = vault_sync::database_path(&db)?;
    drop(db);

    #[cfg(unix)]
    let mut shutdown_signal = ShutdownSignal::arm();

    let runtime = match vault_sync::start_daemon_runtime(db_path.clone()) {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("daemon_start_failed error={err}");
            return Ok(1);
        }
    };

    let pid = std::process::id();
    let http_state = http_config
        .as_ref()
        .map(|c| format!("{}:{}", c.bind, c.port))
        .unwrap_or_else(|| "none".to_string());
    println!(
        "daemon_ready pid={pid} session_id={} db={db_path} http={http_state}",
        runtime.session_id
    );

    let exit_code = if let Some(config) = http_config {
        let db_path_for_factory = db_path.clone();
        let factory =
            move || crate::core::db::open(&db_path_for_factory).map_err(anyhow::Error::from);
        // Block on the SSE listener; the supervisor + worker run in
        // their own threads owned by `runtime`. A supervisor that dies
        // without a shutdown request is a zombie daemon — exit non-zero
        // so the service manager restarts the unit.
        #[cfg(unix)]
        let exit_code = tokio::select! {
            result = crate::mcp::http::run_http(factory, config) => {
                match result {
                    Ok(()) => 0u8,
                    Err(err) => {
                        eprintln!("daemon_http_transport_failed error={err}");
                        1u8
                    }
                }
            }
            () = shutdown_signal.recv() => 0u8,
            exit_code = supervisor_death_watch(&runtime) => exit_code,
        };
        #[cfg(not(unix))]
        let exit_code = tokio::select! {
            result = crate::mcp::http::run_http(factory, config) => {
                match result {
                    Ok(()) => 0u8,
                    Err(err) => {
                        eprintln!("daemon_http_transport_failed error={err}");
                        1u8
                    }
                }
            }
            exit_code = supervisor_death_watch(&runtime) => exit_code,
        };
        exit_code
    } else {
        // No transport: block until a shutdown signal arrives (exit 0)
        // or the supervisor thread dies without one (exit 1).
        #[cfg(unix)]
        let exit_code = wait_for_runtime(&runtime, &mut shutdown_signal).await;
        #[cfg(not(unix))]
        let exit_code = wait_for_runtime(&runtime).await;
        exit_code
    };

    drop(runtime);
    Ok(exit_code)
}

/// Block until either a shutdown signal arrives (normal stop → exit 0)
/// or the supervisor thread exits without one (zombie daemon → exit 1
/// so launchd `KeepAlive` / systemd `Restart=on-failure` restarts the
/// unit instead of leaving a process whose watchers, heartbeats, and
/// extraction have silently stopped).
#[cfg(unix)]
async fn wait_for_runtime(
    runtime: &vault_sync::ServeRuntime,
    shutdown_signal: &mut ShutdownSignal,
) -> u8 {
    tokio::select! {
        () = shutdown_signal.recv() => 0,
        exit_code = supervisor_death_watch(runtime) => exit_code,
    }
}

/// Non-Unix fallback: no signal plumbing, so only the supervisor-death
/// watch ends the foreground task (Ctrl-C tears the binary down).
#[cfg(not(unix))]
async fn wait_for_runtime(runtime: &vault_sync::ServeRuntime) -> u8 {
    supervisor_death_watch(runtime).await
}

/// Polls [`vault_sync::ServeRuntime::supervisor_finished`] every 500 ms
/// and resolves with exit code 1 once the supervisor thread has exited
/// without a shutdown request. Never resolves while the supervisor is
/// healthy (or for transport-only runtimes, which own no supervisor).
async fn supervisor_death_watch(runtime: &vault_sync::ServeRuntime) -> u8 {
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if runtime.supervisor_finished() {
            eprintln!(
                "daemon_supervisor_exited_unexpectedly session_id={} exit_code=1",
                runtime.session_id
            );
            return 1;
        }
    }
}

fn install_action(
    db: Connection,
    http: bool,
    port: Option<u16>,
    bind: Option<IpAddr>,
    token_file: Option<PathBuf>,
    trust_loopback: bool,
) -> Result<u8> {
    let db_path = vault_sync::database_path(&db)?;
    drop(db);

    let unit_args = build_unit_args(&db_path, http, port, bind, token_file, trust_loopback)?;

    install_on_host(&unit_args)?;
    println!("daemon_installed db={db_path}");
    Ok(0)
}

#[cfg(target_os = "macos")]
fn install_on_host(args: &crate::platform::UnitArgs) -> Result<()> {
    crate::platform::launchd::install(args).map_err(anyhow::Error::from)
}

#[cfg(target_os = "linux")]
fn install_on_host(args: &crate::platform::UnitArgs) -> Result<()> {
    crate::platform::systemd::install(args).map_err(anyhow::Error::from)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn install_on_host(_args: &crate::platform::UnitArgs) -> Result<()> {
    Err(anyhow!(
        "quaid daemon install is supported on macOS and Linux only. \
         On this platform, run `quaid daemon run` directly under your preferred \
         supervisor (e.g. nssm on Windows, runit/sysvinit elsewhere)."
    ))
}

fn build_unit_args(
    db_path: &str,
    http: bool,
    port: Option<u16>,
    bind: Option<IpAddr>,
    token_file: Option<PathBuf>,
    trust_loopback: bool,
) -> Result<crate::platform::UnitArgs> {
    let binary_path = std::env::current_exe()
        .map_err(|err| anyhow!("could not resolve current binary path: {err}"))?;
    let http_args = if http {
        Some(crate::platform::UnitHttpArgs {
            port: port.unwrap_or(DEFAULT_HTTP_PORT),
            bind: bind.unwrap_or(DEFAULT_HTTP_BIND),
            token_file,
            trust_loopback,
        })
    } else {
        None
    };
    Ok(crate::platform::UnitArgs {
        binary_path,
        db_path: PathBuf::from(db_path),
        http: http_args,
    })
}

fn uninstall_action() -> Result<u8> {
    uninstall_on_host()?;
    println!("daemon_uninstalled");
    Ok(0)
}

#[cfg(target_os = "macos")]
fn uninstall_on_host() -> Result<()> {
    crate::platform::launchd::uninstall().map_err(anyhow::Error::from)
}

#[cfg(target_os = "linux")]
fn uninstall_on_host() -> Result<()> {
    crate::platform::systemd::uninstall().map_err(anyhow::Error::from)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn uninstall_on_host() -> Result<()> {
    Err(anyhow!(
        "quaid daemon uninstall is supported on macOS and Linux only."
    ))
}

fn start_action() -> Result<u8> {
    start_on_host()?;
    println!("daemon_started");
    Ok(0)
}

#[cfg(target_os = "macos")]
fn start_on_host() -> Result<()> {
    crate::platform::launchd::start().map_err(anyhow::Error::from)
}

#[cfg(target_os = "linux")]
fn start_on_host() -> Result<()> {
    crate::platform::systemd::start().map_err(anyhow::Error::from)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn start_on_host() -> Result<()> {
    Err(anyhow!(
        "quaid daemon start is supported on macOS and Linux only."
    ))
}

fn stop_action() -> Result<u8> {
    stop_on_host()?;
    println!("daemon_stopped");
    Ok(0)
}

#[cfg(target_os = "macos")]
fn stop_on_host() -> Result<()> {
    crate::platform::launchd::stop().map_err(anyhow::Error::from)
}

#[cfg(target_os = "linux")]
fn stop_on_host() -> Result<()> {
    crate::platform::systemd::stop().map_err(anyhow::Error::from)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn stop_on_host() -> Result<()> {
    Err(anyhow!(
        "quaid daemon stop is supported on macOS and Linux only."
    ))
}

fn restart_action() -> Result<u8> {
    stop_on_host()?;
    start_on_host()?;
    println!("daemon_restarted");
    Ok(0)
}

fn status_action(_db: Connection, json: bool) -> Result<u8> {
    let status = status_on_host()?;
    let exit_code = match status {
        crate::platform::UnitStatus::Running => 0u8,
        crate::platform::UnitStatus::InstalledStopped => 1u8,
        crate::platform::UnitStatus::NotInstalled => 2u8,
    };

    if json {
        let payload = serde_json::json!({
            "installed": matches!(
                status,
                crate::platform::UnitStatus::Running | crate::platform::UnitStatus::InstalledStopped
            ),
            "running": matches!(status, crate::platform::UnitStatus::Running),
        });
        println!("{}", serde_json::to_string(&payload)?);
    } else {
        match status {
            crate::platform::UnitStatus::Running => println!("daemon: running"),
            crate::platform::UnitStatus::InstalledStopped => println!("daemon: installed; stopped"),
            crate::platform::UnitStatus::NotInstalled => {
                println!("daemon: not installed — run `quaid daemon install` to set it up");
            }
        }
    }
    Ok(exit_code)
}

#[cfg(target_os = "macos")]
fn status_on_host() -> Result<crate::platform::UnitStatus> {
    crate::platform::launchd::status().map_err(anyhow::Error::from)
}

#[cfg(target_os = "linux")]
fn status_on_host() -> Result<crate::platform::UnitStatus> {
    crate::platform::systemd::status().map_err(anyhow::Error::from)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn status_on_host() -> Result<crate::platform::UnitStatus> {
    Err(anyhow!(
        "quaid daemon status is supported on macOS and Linux only."
    ))
}

fn logs_action(follow: bool, all_streams: bool) -> Result<u8> {
    logs_on_host(follow, all_streams)
}

#[cfg(target_os = "macos")]
fn logs_on_host(follow: bool, all_streams: bool) -> Result<u8> {
    use std::process::Command;

    let err_path = crate::platform::launchd::logs_path_err()?;
    let out_path = crate::platform::launchd::logs_path_out()?;

    if !err_path.exists() && !out_path.exists() {
        eprintln!(
            "no daemon log file found at {} (start the daemon with `quaid daemon start` first)",
            err_path.display()
        );
        return Ok(2);
    }

    let paths: Vec<String> = if all_streams {
        vec![
            out_path.display().to_string(),
            err_path.display().to_string(),
        ]
    } else {
        vec![err_path.display().to_string()]
    };

    let mut cmd = Command::new("tail");
    if follow {
        cmd.arg("-F");
    } else {
        cmd.arg("-n").arg("200");
    }
    for p in &paths {
        cmd.arg(p);
    }
    let status = cmd.status()?;
    Ok(status.code().unwrap_or(1) as u8)
}

#[cfg(target_os = "linux")]
fn logs_on_host(follow: bool, _all_streams: bool) -> Result<u8> {
    use std::process::Command;

    let mut cmd = Command::new("journalctl");
    cmd.arg("--user")
        .arg("-u")
        .arg(crate::platform::systemd::UNIT_NAME);
    if follow {
        cmd.arg("-f");
    } else {
        cmd.arg("-n").arg("200").arg("--no-pager");
    }
    let status = cmd.status()?;
    Ok(status.code().unwrap_or(1) as u8)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn logs_on_host(_follow: bool, _all_streams: bool) -> Result<u8> {
    Err(anyhow!(
        "quaid daemon logs is supported on macOS and Linux only."
    ))
}
