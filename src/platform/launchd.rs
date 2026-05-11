//! macOS launchd integration.
//!
//! Generates `~/Library/LaunchAgents/app.quaid.daemon.plist`, drives
//! `launchctl bootstrap | bootout | kickstart | print` via [`std::process::Command`],
//! and exposes a [`logs_path`] that returns the launchd-captured stderr
//! path so `quaid daemon logs` can tail it without relying on
//! `os_log` subsystem registration.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use super::{argv, home_dir, PlatformError, UnitArgs, UnitStatus, DAEMON_LABEL};

/// Returns `~/Library/LaunchAgents/app.quaid.daemon.plist`.
pub fn plist_path() -> Result<PathBuf, PlatformError> {
    Ok(home_dir()?
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{DAEMON_LABEL}.plist")))
}

/// Returns `~/Library/Logs/quaid-daemon.err.log`. The launchd plist's
/// `StandardErrorPath` points here so the daemon's `eprintln!` lines
/// are captured to a file `quaid daemon logs` can tail with `tail -F`.
pub fn logs_path_err() -> Result<PathBuf, PlatformError> {
    Ok(home_dir()?
        .join("Library")
        .join("Logs")
        .join("quaid-daemon.err.log"))
}

/// Returns `~/Library/Logs/quaid-daemon.out.log`. Captures the daemon's
/// stdout (informational lines) separately so `--all-streams` in
/// `quaid daemon logs` can include both.
pub fn logs_path_out() -> Result<PathBuf, PlatformError> {
    Ok(home_dir()?
        .join("Library")
        .join("Logs")
        .join("quaid-daemon.out.log"))
}

/// Render the plist XML for the given [`UnitArgs`]. Pure function with
/// no I/O so it's straightforward to golden-file test.
pub fn render_plist(args: &UnitArgs) -> Result<String, PlatformError> {
    let argv = argv(args);
    let std_out = logs_path_out()?;
    let std_err = logs_path_err()?;
    let db_env = args.db_path.display().to_string();

    let mut out = String::new();
    out.push_str(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n",
    );
    out.push_str(&format!(
        "    <key>Label</key>\n    <string>{DAEMON_LABEL}</string>\n"
    ));
    out.push_str("    <key>ProgramArguments</key>\n    <array>\n");
    for arg in &argv {
        out.push_str(&format!(
            "        <string>{}</string>\n",
            escape_plist_string(arg)
        ));
    }
    out.push_str("    </array>\n");
    out.push_str("    <key>RunAtLoad</key>\n    <true/>\n");
    out.push_str("    <key>KeepAlive</key>\n    <dict>\n        <key>SuccessfulExit</key>\n        <false/>\n    </dict>\n");
    out.push_str(&format!(
        "    <key>StandardOutPath</key>\n    <string>{}</string>\n",
        escape_plist_string(&std_out.display().to_string())
    ));
    out.push_str(&format!(
        "    <key>StandardErrorPath</key>\n    <string>{}</string>\n",
        escape_plist_string(&std_err.display().to_string())
    ));
    out.push_str("    <key>EnvironmentVariables</key>\n    <dict>\n");
    out.push_str(&format!(
        "        <key>QUAID_DB_PATH</key>\n        <string>{}</string>\n",
        escape_plist_string(&db_env)
    ));
    out.push_str("    </dict>\n");
    out.push_str("</dict>\n</plist>\n");
    Ok(out)
}

fn escape_plist_string(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Resolves the user's uid for the `gui/<uid>` launchctl domain.
fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

/// Ensure `~/Library/Logs/` exists so launchd has somewhere to drop
/// stdout/stderr; if it doesn't, launchd silently swallows the streams.
fn ensure_logs_dir() -> Result<(), PlatformError> {
    let dir = home_dir()?.join("Library").join("Logs");
    fs::create_dir_all(&dir)?;
    Ok(())
}

/// Write the plist and (re)load it with `launchctl bootstrap`. Idempotent:
/// if the unit is already installed, `bootout` it first, then `bootstrap`
/// the regenerated plist.
pub fn install(args: &UnitArgs) -> Result<(), PlatformError> {
    let plist_text = render_plist(args)?;
    let path = plist_path()?;

    ensure_logs_dir()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // If a previous unit is loaded, unload it first; we'll re-bootstrap
    // with the new plist content. Ignore the bootout error if the unit
    // wasn't loaded — `launchctl bootout` returns non-zero in that case
    // and we just want the side effect.
    if path.exists() {
        let _ = launchctl_bootout();
    }

    fs::write(&path, plist_text)?;

    let domain = format!("gui/{}", current_uid());
    let output = Command::new("launchctl")
        .arg("bootstrap")
        .arg(&domain)
        .arg(&path)
        .output()?;
    if !output.status.success() {
        return Err(PlatformError::CommandFailed {
            command: format!("launchctl bootstrap {} {}", domain, path.display()),
            status: output.status.code().unwrap_or(-1),
            stderr: truncate_stderr(&output.stderr),
        });
    }
    Ok(())
}

/// `launchctl bootout` and delete the plist file. Idempotent: returns
/// `Ok(())` even when no plist is present.
pub fn uninstall() -> Result<(), PlatformError> {
    let path = plist_path()?;

    let _ = launchctl_bootout();

    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

fn launchctl_bootout() -> Result<(), PlatformError> {
    let domain = format!("gui/{}", current_uid());
    let target = format!("{}/{}", domain, DAEMON_LABEL);
    let output = Command::new("launchctl")
        .arg("bootout")
        .arg(&target)
        .output()?;
    if !output.status.success() {
        // bootout returns non-zero when the unit isn't loaded — surface as
        // an error so callers can choose to log/ignore but not silently mask.
        return Err(PlatformError::CommandFailed {
            command: format!("launchctl bootout {target}"),
            status: output.status.code().unwrap_or(-1),
            stderr: truncate_stderr(&output.stderr),
        });
    }
    Ok(())
}

/// `launchctl kickstart -k <gui/uid/Label>`. Fails if the unit isn't
/// installed (caller should check via [`status`] first).
pub fn start() -> Result<(), PlatformError> {
    let domain = format!("gui/{}", current_uid());
    let target = format!("{}/{}", domain, DAEMON_LABEL);
    let output = Command::new("launchctl")
        .arg("kickstart")
        .arg("-k")
        .arg(&target)
        .output()?;
    if !output.status.success() {
        return Err(PlatformError::CommandFailed {
            command: format!("launchctl kickstart -k {target}"),
            status: output.status.code().unwrap_or(-1),
            stderr: truncate_stderr(&output.stderr),
        });
    }
    Ok(())
}

/// Stop the daemon via `launchctl bootout`. Returns `Ok(())` even if the
/// unit was already stopped.
pub fn stop() -> Result<(), PlatformError> {
    // bootout removes the unit from launchd's running set. The plist
    // stays on disk so `start`/`status` continue to work.
    let _ = launchctl_bootout();
    Ok(())
}

/// Return the current [`UnitStatus`] by combining plist presence with
/// `launchctl print` exit status.
pub fn status() -> Result<UnitStatus, PlatformError> {
    let path = plist_path()?;
    if !path.exists() {
        return Ok(UnitStatus::NotInstalled);
    }
    let domain = format!("gui/{}", current_uid());
    let target = format!("{}/{}", domain, DAEMON_LABEL);
    let output = Command::new("launchctl")
        .arg("print")
        .arg(&target)
        .output()?;
    if output.status.success() {
        Ok(UnitStatus::Running)
    } else {
        Ok(UnitStatus::InstalledStopped)
    }
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
            db_path: PathBuf::from("/Users/test/.quaid/memory.db"),
            http,
        }
    }

    #[test]
    fn plist_contains_label_and_program_arguments() {
        let plist = render_plist(&sample(None)).unwrap();
        assert!(plist.contains("<string>app.quaid.daemon</string>"));
        assert!(plist.contains("<key>ProgramArguments</key>"));
        assert!(plist.contains("<string>/usr/local/bin/quaid</string>"));
        assert!(plist.contains("<string>daemon</string>"));
        assert!(plist.contains("<string>run</string>"));
    }

    #[test]
    fn plist_includes_log_paths_under_library_logs() {
        let plist = render_plist(&sample(None)).unwrap();
        assert!(plist.contains("<key>StandardOutPath</key>"));
        assert!(plist.contains("quaid-daemon.out.log"));
        assert!(plist.contains("<key>StandardErrorPath</key>"));
        assert!(plist.contains("quaid-daemon.err.log"));
    }

    #[test]
    fn plist_passes_http_flags_through() {
        let http = crate::platform::UnitHttpArgs {
            port: 3112,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            token_file: None,
            trust_loopback: true,
        };
        let plist = render_plist(&sample(Some(http))).unwrap();
        assert!(plist.contains("<string>--http</string>"));
        assert!(plist.contains("<string>--port</string>"));
        assert!(plist.contains("<string>3112</string>"));
        assert!(plist.contains("<string>--bind</string>"));
        assert!(plist.contains("<string>127.0.0.1</string>"));
        assert!(plist.contains("<string>--trust-loopback</string>"));
    }

    #[test]
    fn plist_carries_db_path_as_env_var() {
        let plist = render_plist(&sample(None)).unwrap();
        assert!(plist.contains("<key>EnvironmentVariables</key>"));
        assert!(plist.contains("<key>QUAID_DB_PATH</key>"));
        assert!(plist.contains("/Users/test/.quaid/memory.db"));
    }
}
