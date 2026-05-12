//! Linux systemd user-unit integration.
//!
//! Generates `~/.config/systemd/user/quaid-daemon.service`, drives
//! `systemctl --user daemon-reload | enable --now | stop | restart |
//! disable --now | is-active`. The systemd journal captures stdout/stderr
//! natively so `quaid daemon logs` shells out to `journalctl --user -u
//! quaid-daemon` rather than tailing a file path.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use super::{argv, home_dir, PlatformError, UnitArgs, UnitStatus};

/// Stable systemd unit name. Matches the `--user` namespace.
pub const UNIT_NAME: &str = "quaid-daemon.service";

/// Returns `~/.config/systemd/user/quaid-daemon.service`.
pub fn unit_path() -> Result<PathBuf, PlatformError> {
    Ok(home_dir()?
        .join(".config")
        .join("systemd")
        .join("user")
        .join(UNIT_NAME))
}

/// Render the systemd `.service` text for the given [`UnitArgs`]. Pure
/// function with no I/O; used by [`install`] and exercised by
/// golden-file tests.
pub fn render_unit(args: &UnitArgs) -> String {
    let exec_start = argv(args)
        .into_iter()
        .map(quote_for_exec_start)
        .collect::<Vec<_>>()
        .join(" ");

    let mut out = String::new();
    out.push_str("[Unit]\n");
    out.push_str("Description=Quaid personal AI memory daemon\n");
    out.push_str("After=network-online.target\n\n");
    out.push_str("[Service]\n");
    out.push_str("Type=simple\n");
    out.push_str(&format!("ExecStart={exec_start}\n"));
    out.push_str(&format!(
        "Environment=QUAID_DB_PATH={}\n",
        args.db_path.display()
    ));
    out.push_str("Restart=on-failure\n");
    out.push_str("RestartSec=5\n\n");
    out.push_str("[Install]\n");
    out.push_str("WantedBy=default.target\n");
    out
}

/// Shell-quote a single ExecStart argument. systemd accepts simple
/// space-separated tokens; values containing spaces, quotes, or
/// backslashes need double-quoting with `\\` and `\"` escaping per
/// `systemd.service(5)`.
fn quote_for_exec_start(s: String) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | '=' | ','))
    {
        s
    } else {
        let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    }
}

/// Write the unit file and `systemctl --user daemon-reload && enable --now`.
/// Idempotent across reruns: if the unit already exists, the new content
/// overwrites and the service is restarted to pick up flag changes.
pub fn install(args: &UnitArgs) -> Result<(), PlatformError> {
    let unit_text = render_unit(args);
    let path = unit_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let already_installed = path.exists();
    fs::write(&path, unit_text)?;

    run_systemctl(&["daemon-reload"])?;
    if already_installed {
        // Pre-existing unit: reload picked up the new ExecStart; restart
        // the service so the running daemon honors the new flags.
        run_systemctl(&["restart", UNIT_NAME])?;
    } else {
        run_systemctl(&["enable", "--now", UNIT_NAME])?;
    }
    Ok(())
}

/// `systemctl --user disable --now`, delete the unit, then `daemon-reload`.
/// Idempotent: succeeds even if no unit was installed.
pub fn uninstall() -> Result<(), PlatformError> {
    let path = unit_path()?;
    if path.exists() {
        // `disable --now` stops AND disables in one go.
        let _ = run_systemctl(&["disable", "--now", UNIT_NAME]);
        fs::remove_file(&path)?;
        run_systemctl(&["daemon-reload"])?;
    }
    Ok(())
}

/// `systemctl --user start`.
pub fn start() -> Result<(), PlatformError> {
    run_systemctl(&["start", UNIT_NAME])
}

/// `systemctl --user stop`. Returns successfully even if the unit
/// wasn't running.
pub fn stop() -> Result<(), PlatformError> {
    // `stop` exits non-zero if the unit isn't loaded; mask that.
    let _ = run_systemctl(&["stop", UNIT_NAME]);
    Ok(())
}

/// Returns the current [`UnitStatus`] by combining file presence with
/// `systemctl --user is-active` exit status.
pub fn status() -> Result<UnitStatus, PlatformError> {
    let path = unit_path()?;
    if !path.exists() {
        return Ok(UnitStatus::NotInstalled);
    }
    let output = Command::new("systemctl")
        .arg("--user")
        .arg("is-active")
        .arg(UNIT_NAME)
        .output()?;
    // `is-active` returns 0 for active, 3 for inactive/failed/dead.
    if output.status.success() {
        Ok(UnitStatus::Running)
    } else {
        Ok(UnitStatus::InstalledStopped)
    }
}

fn run_systemctl(args: &[&str]) -> Result<(), PlatformError> {
    let output = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(PlatformError::CommandFailed {
            command: format!("systemctl --user {}", args.join(" ")),
            status: output.status.code().unwrap_or(-1),
            stderr: truncate_stderr(&output.stderr),
        });
    }
    Ok(())
}

fn truncate_stderr(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    if s.len() > 512 {
        format!("{}…", &s[..512])
    } else {
        s.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn sample(http: Option<crate::platform::UnitHttpArgs>) -> UnitArgs {
        UnitArgs {
            binary_path: PathBuf::from("/usr/local/bin/quaid"),
            db_path: PathBuf::from("/home/test/.quaid/memory.db"),
            http,
        }
    }

    #[test]
    fn unit_text_contains_required_sections() {
        let unit = render_unit(&sample(None));
        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("[Service]"));
        assert!(unit.contains("Type=simple"));
        assert!(unit.contains("ExecStart=/usr/local/bin/quaid daemon run"));
        assert!(unit.contains("Environment=QUAID_DB_PATH=/home/test/.quaid/memory.db"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("[Install]"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn unit_text_passes_http_flags_in_exec_start() {
        let http = crate::platform::UnitHttpArgs {
            port: 3112,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            token_file: None,
            trust_loopback: false,
        };
        let unit = render_unit(&sample(Some(http)));
        assert!(unit.contains("--http"));
        assert!(unit.contains("--port 3112"));
        assert!(unit.contains("--bind 127.0.0.1"));
        assert!(!unit.contains("--trust-loopback"));
    }

    #[test]
    fn unit_text_quotes_paths_with_spaces() {
        let args = UnitArgs {
            binary_path: PathBuf::from("/Applications/My Quaid.app/quaid"),
            db_path: PathBuf::from("/home/test/.quaid/memory.db"),
            http: None,
        };
        let unit = render_unit(&args);
        assert!(unit.contains("\"/Applications/My Quaid.app/quaid\""));
    }
}
