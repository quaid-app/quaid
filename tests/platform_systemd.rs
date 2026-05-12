#![cfg(target_os = "linux")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test fixtures legitimately panic on setup failure; per-site #[expect] would add noise"
)]

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use quaid::platform::{systemd, PlatformError, UnitArgs, UnitHttpArgs, UnitStatus};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> &'static Mutex<()> {
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: OsString) -> Self {
        let previous = std::env::var_os(key);
        #[expect(
            unsafe_code,
            reason = "std::env::set_var is unsafe on Rust 1.81+; tests hold ENV_LOCK while mutating process environment"
        )]
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        #[expect(
            unsafe_code,
            reason = "std::env::set_var/remove_var are unsafe on Rust 1.81+; guard restores while ENV_LOCK is held"
        )]
        unsafe {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}

struct FakeSystemctl {
    _dir: tempfile::TempDir,
    log_path: PathBuf,
    status_exit_path: PathBuf,
    fail_all_path: PathBuf,
    _home: EnvGuard,
    _path: EnvGuard,
    _log: EnvGuard,
    _status_exit: EnvGuard,
    _fail_all: EnvGuard,
}

impl FakeSystemctl {
    fn new() -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        let bin_dir = dir.path().join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        write_fake_systemctl(&bin_dir.join("systemctl"));

        let old_path = std::env::var_os("PATH").unwrap_or_default();
        let mut path = OsString::from(bin_dir.as_os_str());
        path.push(":");
        path.push(old_path);

        let log_path = dir.path().join("systemctl.log");
        let status_exit_path = dir.path().join("status-exit");
        let fail_all_path = dir.path().join("fail-all");

        let home = EnvGuard::set("HOME", dir.path().as_os_str().to_os_string());
        let path_guard = EnvGuard::set("PATH", path);
        let log = EnvGuard::set("SYSTEMCTL_LOG", log_path.as_os_str().to_os_string());
        let status_exit = EnvGuard::set(
            "SYSTEMCTL_STATUS_EXIT",
            status_exit_path.as_os_str().to_os_string(),
        );
        let fail_all = EnvGuard::set(
            "SYSTEMCTL_FAIL_ALL",
            fail_all_path.as_os_str().to_os_string(),
        );

        Self {
            _dir: dir,
            log_path,
            status_exit_path,
            fail_all_path,
            _home: home,
            _path: path_guard,
            _log: log,
            _status_exit: status_exit,
            _fail_all: fail_all,
        }
    }

    fn log(&self) -> String {
        fs::read_to_string(&self.log_path).unwrap_or_default()
    }

    fn set_status_exit(&self, code: i32) {
        fs::write(&self.status_exit_path, code.to_string()).unwrap();
    }

    fn fail_all_commands(&self, stderr: &str) {
        fs::write(&self.fail_all_path, stderr).unwrap();
    }
}

fn write_fake_systemctl(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(
        path,
        "#!/bin/sh\n\
         printf '%s\\n' \"$*\" >> \"$SYSTEMCTL_LOG\"\n\
         if [ -f \"$SYSTEMCTL_FAIL_ALL\" ]; then\n\
           cat \"$SYSTEMCTL_FAIL_ALL\" >&2\n\
           exit 42\n\
         fi\n\
         if [ \"$2\" = \"is-active\" ] && [ -f \"$SYSTEMCTL_STATUS_EXIT\" ]; then\n\
           exit \"$(cat \"$SYSTEMCTL_STATUS_EXIT\")\"\n\
         fi\n\
         exit 0\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn sample(http: Option<UnitHttpArgs>) -> UnitArgs {
    UnitArgs {
        binary_path: PathBuf::from("/usr/local/bin/quaid"),
        db_path: PathBuf::from("/home/test/.quaid/memory.db"),
        http,
    }
}

#[test]
fn render_unit_omits_optional_http_flags_when_unset() {
    let http = UnitHttpArgs {
        port: 3113,
        bind: "127.0.0.1".parse().unwrap(),
        token_file: None,
        trust_loopback: false,
    };

    let unit = systemd::render_unit(&sample(Some(http)));

    assert!(unit.contains("--http"));
    assert!(!unit.contains("--token-file"));
    assert!(!unit.contains("--trust-loopback"));
}

#[test]
fn render_unit_quotes_paths_with_quotes_and_backslashes() {
    let args = UnitArgs {
        binary_path: PathBuf::from("/opt/Quaid \"Nightly\"\\quaid"),
        db_path: PathBuf::from("/home/test/.quaid/memory.db"),
        http: None,
    };

    let unit = systemd::render_unit(&args);

    assert!(unit.contains("\"/opt/Quaid \\\"Nightly\\\"\\\\quaid\""));
}

#[test]
fn install_writes_unit_and_reinstall_restarts_existing_unit() {
    let _guard = env_lock().lock().unwrap();
    let fake = FakeSystemctl::new();

    systemd::install(&sample(None)).unwrap();
    systemd::install(&sample(None)).unwrap();

    let unit = fs::read_to_string(systemd::unit_path().unwrap()).unwrap();
    assert!(unit.contains("ExecStart=/usr/local/bin/quaid daemon run"));
    let log = fake.log();
    assert!(log.contains("--user daemon-reload"));
    assert!(log.contains("--user enable --now quaid-daemon.service"));
    assert!(log.contains("--user restart quaid-daemon.service"));
}

#[test]
fn status_distinguishes_not_installed_running_and_stopped() {
    let _guard = env_lock().lock().unwrap();
    let fake = FakeSystemctl::new();

    assert_eq!(systemd::status().unwrap(), UnitStatus::NotInstalled);

    let path = systemd::unit_path().unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, systemd::render_unit(&sample(None))).unwrap();

    fake.set_status_exit(0);
    assert_eq!(systemd::status().unwrap(), UnitStatus::Running);

    fake.set_status_exit(3);
    assert_eq!(systemd::status().unwrap(), UnitStatus::InstalledStopped);
}

#[test]
fn lifecycle_commands_shell_out_and_uninstall_removes_unit() {
    let _guard = env_lock().lock().unwrap();
    let fake = FakeSystemctl::new();
    let path = systemd::unit_path().unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, systemd::render_unit(&sample(None))).unwrap();

    systemd::start().unwrap();
    systemd::stop().unwrap();
    systemd::uninstall().unwrap();

    assert!(!path.exists());
    let log = fake.log();
    assert!(log.contains("--user start quaid-daemon.service"));
    assert!(log.contains("--user stop quaid-daemon.service"));
    assert!(log.contains("--user disable --now quaid-daemon.service"));
    assert!(log.contains("--user daemon-reload"));
}

#[test]
fn command_failure_surfaces_command_status_and_stderr() {
    let _guard = env_lock().lock().unwrap();
    let fake = FakeSystemctl::new();
    fake.fail_all_commands("boom");

    let error = systemd::start().unwrap_err();

    match error {
        PlatformError::CommandFailed {
            command,
            status,
            stderr,
        } => {
            assert_eq!(command, "systemctl --user start quaid-daemon.service");
            assert_eq!(status, 42);
            assert_eq!(stderr, "boom");
        }
        other => panic!("expected CommandFailed, got {other:?}"),
    }
}
