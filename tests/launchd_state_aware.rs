#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration tests panic on setup failure"
)]

//! State-aware launchd lifecycle driven through a *stateful*
//! PATH-stubbed `launchctl` (`bootstrap` loads, `bootout` unloads and
//! fails when already unloaded, `kickstart -k` fails when unloaded,
//! `print`'s exit status reports load state — the same contract as
//! real launchd). Runs on Linux CI because `src/platform/launchd.rs`
//! is compiled on all Unix targets for exactly this purpose.
//!
//! Regression background (review §18): `stop()` boots the unit out,
//! but the old `start()` ran `kickstart -k` unconditionally, which
//! errors on an unloaded unit — so on macOS every `stop → start` and
//! `daemon restart` (stop + start) failed. `start()` must probe load
//! state via `launchctl print`'s exit status and `bootstrap` the
//! on-disk plist when the unit is not loaded.

#[path = "common/vault_sync_fixtures.rs"]
mod fixtures;

use std::fs;
use std::path::Path;
use std::process::Command;

use quaid::platform::{launchd, UnitArgs, UnitStatus};

/// Mirrors real launchctl semantics keyed off a state file:
/// loaded/unloaded transitions plus per-verb failure modes.
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

fn write_stub(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, STATEFUL_LAUNCHCTL_STUB).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[test]
fn launchd_start_is_state_aware_across_stop_and_restart() {
    let _lock = fixtures::env_mutation_lock().lock().unwrap();

    let dir = tempfile::TempDir::new().unwrap();
    let home = dir.path().join("home");
    let bin = dir.path().join("bin");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&bin).unwrap();
    write_stub(&bin.join("launchctl"));

    let log_path = dir.path().join("platform.log");
    let state_path = dir.path().join("launchctl.state");

    let path_value = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let _path = fixtures::EnvVarGuard::set("PATH", &path_value);
    let _home = fixtures::EnvVarGuard::set("HOME", home.to_str().unwrap());
    let _log = fixtures::EnvVarGuard::set("FAKE_PLATFORM_LOG", log_path.to_str().unwrap());
    let _state = fixtures::EnvVarGuard::set("FAKE_LAUNCHCTL_STATE", state_path.to_str().unwrap());

    let read_state = || {
        fs::read_to_string(&state_path)
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "unloaded".to_string())
    };
    let read_log = || fs::read_to_string(&log_path).unwrap_or_default();

    // The old verb mix is invalid under a faithful launchctl state
    // machine: `kickstart -k` on an unloaded service fails — this is
    // what the pre-fix `start()` ran unconditionally after `stop`'s
    // bootout, breaking every macOS stop→start and restart.
    let kickstart_unloaded = Command::new("launchctl")
        .args(["kickstart", "-k", "gui/501/app.quaid.daemon"])
        // Separate log so the sanity probe doesn't skew the verb counts
        // asserted against the production-code invocations below.
        .env("FAKE_PLATFORM_LOG", dir.path().join("sanity.log"))
        .output()
        .unwrap();
    assert!(
        !kickstart_unloaded.status.success(),
        "stub sanity: kickstart on an unloaded service must fail"
    );

    let args = UnitArgs {
        binary_path: "/usr/local/bin/quaid".into(),
        db_path: home.join("memory.db"),
        http: None,
    };

    // install: writes the plist and bootstraps the unit (loaded).
    launchd::install(&args).unwrap();
    assert_eq!(read_state(), "loaded");
    assert!(matches!(launchd::status().unwrap(), UnitStatus::Running));

    // start while loaded → print probe + kickstart, no second bootstrap.
    launchd::start().unwrap();
    let log = read_log();
    assert_eq!(log.matches("launchctl:kickstart -k").count(), 1);
    assert_eq!(log.matches("launchctl:bootstrap").count(), 1);

    // stop boots the unit out; the plist stays on disk for later starts.
    launchd::stop().unwrap();
    assert_eq!(read_state(), "unloaded");
    assert!(matches!(
        launchd::status().unwrap(),
        UnitStatus::InstalledStopped
    ));

    // start after stop must probe-and-bootstrap; the old kickstart-only
    // start fails exactly here (proven by the stub sanity check above).
    launchd::start().unwrap();
    assert_eq!(read_state(), "loaded");
    let log = read_log();
    assert_eq!(
        log.matches("launchctl:bootstrap").count(),
        2,
        "start on an unloaded unit must bootstrap the on-disk plist"
    );
    assert_eq!(
        log.matches("launchctl:kickstart -k").count(),
        1,
        "the unloaded branch must not attempt kickstart"
    );

    // The stop + start chain `quaid daemon restart` issues end-to-end:
    // bootout (unloads) then state-aware start (bootstraps again).
    launchd::stop().unwrap();
    launchd::start().unwrap();
    assert_eq!(read_state(), "loaded");
    assert!(matches!(launchd::status().unwrap(), UnitStatus::Running));
    let log = read_log();
    assert_eq!(log.matches("launchctl:bootstrap").count(), 3);
    assert!(
        log.matches("launchctl:print").count() >= 3,
        "every start and status call must probe load state via print"
    );
}
