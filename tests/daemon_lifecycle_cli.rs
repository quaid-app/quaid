#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and inspect subprocess output"
)]

mod common;
#[path = "common/subprocess.rs"]
mod common_subprocess;

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use quaid::core::db;
use serde_json::Value;

/// Stateful fake `launchctl`: `bootstrap` marks the unit loaded,
/// `bootout` marks it unloaded (failing when it already is, like real
/// launchd), `kickstart -k` fails on an unloaded unit, and `print`'s
/// exit status reports load state. This is the contract that made the
/// old kickstart-only `start()` fail after every `stop` — an
/// unconditional `exit 0` stub could never catch that verb mismatch.
#[cfg(target_os = "macos")]
const STATEFUL_LAUNCHCTL_STUB: &str = r#"#!/bin/sh
printf 'launchctl:%s\n' "$*" >> "$FAKE_PLATFORM_LOG"
state() { cat "$FAKE_LAUNCHCTL_STATE" 2>/dev/null || echo unloaded; }
case "$1" in
  print)
    [ "$(state)" = "loaded" ] && exit 0
    echo "Could not find service \"app.quaid.daemon\" in domain for user gui" >&2
    exit 113
    ;;
  bootstrap)
    if [ "$(state)" = "loaded" ]; then
      echo "Bootstrap failed: 17: File exists" >&2
      exit 17
    fi
    echo loaded > "$FAKE_LAUNCHCTL_STATE"
    exit 0
    ;;
  bootout)
    if [ "$(state)" = "loaded" ]; then
      echo unloaded > "$FAKE_LAUNCHCTL_STATE"
      exit 0
    fi
    echo "Boot-out failed: 3: No such process" >&2
    exit 3
    ;;
  kickstart)
    [ "$(state)" = "loaded" ] && exit 0
    echo "Could not find service \"app.quaid.daemon\" in domain for user gui" >&2
    exit 113
    ;;
esac
exit 0
"#;

struct FakePlatform {
    _dir: tempfile::TempDir,
    home_dir: PathBuf,
    path_env: OsString,
    log_path: PathBuf,
    state_path: PathBuf,
}

impl FakePlatform {
    fn new() -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        let home_dir = dir.path().join("home");
        let bin_dir = dir.path().join("bin");
        fs::create_dir_all(&home_dir).unwrap();
        fs::create_dir_all(&bin_dir).unwrap();

        let log_path = dir.path().join("platform.log");
        let state_path = dir.path().join("launchctl.state");
        #[cfg(target_os = "macos")]
        write_fake_command(&bin_dir.join("launchctl"), STATEFUL_LAUNCHCTL_STUB);
        #[cfg(target_os = "linux")]
        {
            write_fake_command(
                &bin_dir.join("systemctl"),
                "#!/bin/sh\nprintf 'systemctl:%s\\n' \"$*\" >> \"$FAKE_PLATFORM_LOG\"\nif [ \"$2\" = \"is-active\" ]; then exit 0; fi\nexit 0\n",
            );
            write_fake_command(
                &bin_dir.join("journalctl"),
                "#!/bin/sh\nprintf 'journalctl:%s\\n' \"$*\" >> \"$FAKE_PLATFORM_LOG\"\nexit 0\n",
            );
        }

        let old_path = std::env::var_os("PATH").unwrap_or_default();
        let mut path_env = OsString::from(bin_dir.as_os_str());
        path_env.push(":");
        path_env.push(old_path);

        Self {
            _dir: dir,
            home_dir,
            path_env,
            log_path,
            state_path,
        }
    }

    fn command(&self, db_path: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(common::quaid_bin());
        common_subprocess::configure_test_command(&mut command);
        command
            .env("HOME", &self.home_dir)
            .env("USERPROFILE", &self.home_dir)
            .env("PATH", &self.path_env)
            .env("FAKE_PLATFORM_LOG", &self.log_path)
            .env("FAKE_LAUNCHCTL_STATE", &self.state_path)
            .arg("--db")
            .arg(db_path)
            .args(args);
        command
    }

    fn run(&self, db_path: &Path, args: &[&str]) -> Output {
        self.command(db_path, args).output().expect("run quaid")
    }

    fn log(&self) -> String {
        fs::read_to_string(&self.log_path).unwrap_or_default()
    }

    #[cfg(target_os = "macos")]
    fn unit_text(&self) -> String {
        fs::read_to_string(
            self.home_dir
                .join("Library")
                .join("LaunchAgents")
                .join("app.quaid.daemon.plist"),
        )
        .unwrap()
    }

    #[cfg(target_os = "linux")]
    fn unit_text(&self) -> String {
        fs::read_to_string(
            self.home_dir
                .join(".config")
                .join("systemd")
                .join("user")
                .join("quaid-daemon.service"),
        )
        .unwrap()
    }

    #[cfg(target_os = "macos")]
    fn seed_log_file(&self) {
        let logs_dir = self.home_dir.join("Library").join("Logs");
        fs::create_dir_all(&logs_dir).unwrap();
        fs::write(logs_dir.join("quaid-daemon.err.log"), "daemon log line\n").unwrap();
    }
}

#[cfg(unix)]
fn write_fake_command(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, script).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn open_test_db_file(dir: &tempfile::TempDir) -> PathBuf {
    let db_path = fs::canonicalize(dir.path()).unwrap().join("memory.db");
    db::open(db_path.to_str().unwrap()).unwrap();
    db_path
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn daemon_install_writes_unit_with_http_flags_and_status_reports_running() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = open_test_db_file(&dir);
    let platform = FakePlatform::new();

    let install = platform.run(
        &db_path,
        &[
            "daemon",
            "install",
            "--http",
            "--port",
            "4000",
            "--bind",
            "127.0.0.1",
            "--trust-loopback",
        ],
    );
    assert!(
        install.status.success(),
        "install stderr: {}",
        String::from_utf8_lossy(&install.stderr)
    );

    let unit = platform.unit_text();
    assert!(unit.contains("daemon"));
    assert!(unit.contains("run"));
    assert!(unit.contains("--http"));
    assert!(unit.contains("4000"));
    assert!(unit.contains("127.0.0.1"));
    assert!(unit.contains("--trust-loopback"));

    let status = platform.run(&db_path, &["daemon", "status", "--json"]);
    assert_eq!(status.status.code(), Some(0));
    let payload: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(payload["installed"], true);
    assert_eq!(payload["running"], true);

    let log = platform.log();
    #[cfg(target_os = "macos")]
    assert!(log.contains("launchctl:bootstrap"));
    #[cfg(target_os = "linux")]
    assert!(log.contains("systemctl:--user enable --now quaid-daemon.service"));
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn daemon_start_stop_restart_and_logs_dispatch_to_platform() {
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = open_test_db_file(&dir);
    let platform = FakePlatform::new();

    let install = platform.run(&db_path, &["daemon", "install"]);
    assert!(install.status.success());

    for (args, expected_stdout) in [
        (&["daemon", "start"][..], "daemon_started"),
        (&["daemon", "stop"][..], "daemon_stopped"),
        (&["daemon", "restart"][..], "daemon_restarted"),
    ] {
        let output = platform.run(&db_path, args);
        assert!(
            output.status.success(),
            "{args:?} stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains(expected_stdout));
    }

    #[cfg(target_os = "macos")]
    platform.seed_log_file();
    let logs = platform.run(&db_path, &["daemon", "logs"]);
    assert!(
        logs.status.success(),
        "logs stderr: {}",
        String::from_utf8_lossy(&logs.stderr)
    );

    let log = platform.log();
    #[cfg(target_os = "macos")]
    {
        // install loaded the unit (bootstrap #1); `start` saw it loaded
        // (print) and kickstarted; `stop` booted it out; `restart` =
        // stop (bootout on an unloaded unit fails and is ignored) +
        // start, which saw the unit unloaded and bootstrapped again
        // (#2). The pre-fix kickstart-only start fails that last arm
        // against this stateful stub, which is the macOS restart bug.
        assert!(log.contains("launchctl:print"));
        assert_eq!(
            log.matches("launchctl:kickstart -k").count(),
            1,
            "only the loaded-state start may kickstart"
        );
        assert_eq!(
            log.matches("launchctl:bootstrap").count(),
            2,
            "install and the restart-after-stop start must both bootstrap"
        );
        assert!(log.contains("launchctl:bootout"));
    }
    #[cfg(target_os = "linux")]
    {
        assert!(log.contains("systemctl:--user start quaid-daemon.service"));
        assert!(log.contains("systemctl:--user stop quaid-daemon.service"));
        assert_eq!(
            log.matches("systemctl:--user start quaid-daemon.service")
                .count(),
            2,
            "restart dispatch should issue a second start after stop"
        );
        assert_eq!(
            log.matches("systemctl:--user stop quaid-daemon.service")
                .count(),
            2,
            "restart dispatch should issue a second stop before start"
        );
        assert!(log.contains("journalctl:--user -u quaid-daemon.service -n 200 --no-pager"));
    }
}
