//! Platform-native unit generators and service-manager helpers.
//!
//! Backs `quaid daemon install | uninstall | start | stop | restart |
//! status | logs`. macOS uses launchd (plist files under
//! `~/Library/LaunchAgents/` and `launchctl bootstrap/bootout`), Linux
//! uses systemd user units (`~/.config/systemd/user/` and
//! `systemctl --user`). Other platforms surface a typed
//! [`PlatformError::Unsupported`] so the CLI can exit non-zero with a
//! documented "manual setup" message.
//!
//! Module layout:
//! - `launchd` (macOS at runtime; compiled on all Unix so the
//!   `launchctl` state machine stays covered by PATH-stubbed
//!   integration tests on Linux CI) — plist generator, `launchctl` wrappers
//! - `systemd` (Linux only) — unit generator, `systemctl --user` wrappers
//! - [`UnitArgs`] — the shared argument shape both generators consume

#[cfg(unix)]
pub mod launchd;
#[cfg(target_os = "linux")]
pub mod systemd;

use std::path::PathBuf;

use thiserror::Error;

/// Stable identifier (used in plist `Label`, systemd unit name, and
/// log files) so the install/uninstall/status paths agree on names.
pub const DAEMON_LABEL: &str = "app.quaid.daemon";

/// Arguments threaded into both platform unit generators. Captured by
/// `quaid daemon install` from `current_exe()` + the active database
/// path + the operator's optional HTTP flags.
#[derive(Debug, Clone)]
pub struct UnitArgs {
    /// Absolute path to the `quaid` binary that should be exec'd by
    /// the service manager.
    pub binary_path: PathBuf,
    /// Absolute path to the database the daemon should attach to.
    pub db_path: PathBuf,
    /// `Some(http)` to bake `--http` plus its sub-flags into the
    /// generated unit's argv; `None` for stdio-less workers-only daemons.
    pub http: Option<UnitHttpArgs>,
}

/// HTTP transport flags propagated from `quaid daemon install` into
/// the generated unit's argv. Maps 1:1 to the `HttpConfig` fields the
/// daemon reads at startup.
#[derive(Debug, Clone)]
pub struct UnitHttpArgs {
    /// TCP port (e.g. `3112`).
    pub port: u16,
    /// Bind address (defaults to `127.0.0.1`).
    pub bind: std::net::IpAddr,
    /// Optional path to a bearer-token file. Currently advisory only
    /// in v1 since bearer-auth enforcement is deferred.
    pub token_file: Option<PathBuf>,
    /// `true` if the unit should also pass `--trust-loopback`.
    pub trust_loopback: bool,
}

/// Errors raised by unit generation or by shelling out to the platform
/// service-manager binaries.
#[derive(Debug, Error)]
pub enum PlatformError {
    /// The host platform doesn't have a supported native service manager.
    #[error("daemon lifecycle commands are not supported on this platform; see operator docs for manual setup options ({reason})")]
    Unsupported {
        /// Static reason string suitable for surface-level user diagnostics.
        reason: &'static str,
    },

    /// The plist/unit could not be written to disk.
    #[error("failed to write unit file: {message}")]
    WriteFailed {
        /// Description of the underlying I/O failure.
        message: String,
    },

    /// A platform binary (`launchctl`, `systemctl`) failed.
    #[error("platform command failed: {command} (exit={status}): {stderr}")]
    CommandFailed {
        /// The shelled-out command line (sanitized for logging).
        command: String,
        /// The command's exit status, or `-1` if the process couldn't
        /// be spawned at all.
        status: i32,
        /// stderr text (truncated to ~512 bytes for safety).
        stderr: String,
    },

    /// `HOME` couldn't be resolved (no `$HOME`, no fallback). Should be
    /// vanishingly rare on macOS/Linux; surfaced for completeness.
    #[error("could not resolve home directory: {message}")]
    NoHome {
        /// Underlying cause from `dirs::home_dir`/`std::env::var`.
        message: String,
    },
}

impl From<std::io::Error> for PlatformError {
    fn from(err: std::io::Error) -> Self {
        PlatformError::WriteFailed {
            message: err.to_string(),
        }
    }
}

/// Resolve the user's home directory in a way both macOS and Linux
/// agree on. Returns [`PlatformError::NoHome`] if it cannot be found.
pub fn home_dir() -> Result<PathBuf, PlatformError> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| PlatformError::NoHome {
            message: "HOME environment variable is not set".to_string(),
        })
}

/// Status of the installed daemon unit on the current platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitStatus {
    /// Unit file is present and the service manager reports it active.
    Running,
    /// Unit file is present but the service is not running.
    InstalledStopped,
    /// No unit file is present at the expected platform path.
    NotInstalled,
}

/// Renders [`UnitArgs`] into the `ProgramArguments` / `ExecStart`
/// command-line tokens shared by both unit generators.
pub(crate) fn argv(args: &UnitArgs) -> Vec<String> {
    let mut argv: Vec<String> = vec![
        args.binary_path.display().to_string(),
        "daemon".to_string(),
        "run".to_string(),
    ];
    if let Some(http) = &args.http {
        argv.push("--http".to_string());
        argv.push("--port".to_string());
        argv.push(http.port.to_string());
        argv.push("--bind".to_string());
        argv.push(http.bind.to_string());
        if let Some(path) = &http.token_file {
            argv.push("--token-file".to_string());
            argv.push(path.display().to_string());
        }
        if http.trust_loopback {
            argv.push("--trust-loopback".to_string());
        }
    }
    argv
}

/// Stub used on platforms with no native service-manager integration.
/// Always returns [`PlatformError::Unsupported`].
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn unsupported<T>(reason: &'static str) -> Result<T, PlatformError> {
    Err(PlatformError::Unsupported { reason })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use std::path::PathBuf;

    fn sample_args(http: Option<UnitHttpArgs>) -> UnitArgs {
        UnitArgs {
            binary_path: PathBuf::from("/usr/local/bin/quaid"),
            db_path: PathBuf::from("/Users/test/.quaid/memory.db"),
            http,
        }
    }

    #[test]
    fn argv_without_http_just_invokes_daemon_run() {
        let v = argv(&sample_args(None));
        assert_eq!(
            v,
            vec![
                "/usr/local/bin/quaid".to_string(),
                "daemon".to_string(),
                "run".to_string(),
            ]
        );
    }

    #[test]
    fn argv_with_http_passes_all_flags_through() {
        let http = UnitHttpArgs {
            port: 3112,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            token_file: Some(PathBuf::from("/Users/test/.quaid/http_token")),
            trust_loopback: true,
        };
        let v = argv(&sample_args(Some(http)));
        assert!(v.iter().any(|s| s == "--http"));
        assert!(v.iter().any(|s| s == "--port"));
        assert!(v.iter().any(|s| s == "3112"));
        assert!(v.iter().any(|s| s == "--bind"));
        assert!(v.iter().any(|s| s == "127.0.0.1"));
        assert!(v.iter().any(|s| s == "--token-file"));
        assert!(v.iter().any(|s| s == "/Users/test/.quaid/http_token"));
        assert!(v.iter().any(|s| s == "--trust-loopback"));
    }
}
