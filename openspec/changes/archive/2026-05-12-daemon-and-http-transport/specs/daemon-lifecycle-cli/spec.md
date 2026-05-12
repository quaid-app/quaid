## ADDED Requirements

### Requirement: `quaid daemon install` generates a platform-native unit and registers it

The system SHALL provide a `quaid daemon install` subcommand that generates a platform-native service definition that invokes `quaid daemon run` against the active database. On macOS the system SHALL write `~/Library/LaunchAgents/app.quaid.daemon.plist` with `StandardOutPath` set to `~/Library/Logs/quaid-daemon.out.log` and `StandardErrorPath` set to `~/Library/Logs/quaid-daemon.err.log`, then bootstrap it via `launchctl bootstrap gui/$UID <plist>`. On Linux the system SHALL write `~/.config/systemd/user/quaid-daemon.service`, run `systemctl --user daemon-reload`, and enable+start the unit via `systemctl --user enable --now quaid-daemon`. The generated unit SHALL include the absolute path to the current `quaid` binary (resolved via `std::env::current_exe()` at install time) and the absolute path to the active database. On unsupported platforms the command SHALL exit non-zero with a documented error explaining manual setup options.

#### Scenario: macOS install writes plist with log paths and bootstraps via launchctl
- **WHEN** `quaid daemon install` is invoked on macOS with no flags
- **THEN** `~/Library/LaunchAgents/app.quaid.daemon.plist` is written with `StandardOutPath` and `StandardErrorPath` keys pointing at `~/Library/Logs/quaid-daemon.{out,err}.log`
- **AND** the plist passes `plutil -lint`
- **AND** `launchctl bootstrap gui/<uid> <plist>` is invoked
- **AND** the service is running within 2 seconds of the install completing

#### Scenario: Linux install writes systemd unit and starts via systemctl
- **WHEN** `quaid daemon install` is invoked on Linux with no flags
- **THEN** `~/.config/systemd/user/quaid-daemon.service` is written
- **AND** the unit passes `systemd-analyze --user verify`
- **AND** `systemctl --user daemon-reload` is run, followed by `systemctl --user enable --now quaid-daemon`
- **AND** the unit's `ActiveState` is `active` within 2 seconds

#### Scenario: Unsupported platform exits with manual-setup guidance
- **WHEN** `quaid daemon install` is invoked on Windows or another unsupported OS
- **THEN** the command exits with status 2 and a message naming the supported platforms
- **AND** the message points to operator docs for manual service setup

### Requirement: `quaid daemon install` is idempotent and accepts transport flags that flow to the daemon

`quaid daemon install` SHALL accept optional flags `--http`, `--port <N>`, `--bind <addr>`, `--token-file <path>`, and `--trust-loopback` that are passed through to the unit's `ExecStart` / `ProgramArguments` so the installed daemon hosts the HTTP/SSE MCP transport directly when those flags are supplied (per the `mcp-http-transport` capability and the daemon-runtime "may host HTTP/SSE" requirement). Reruns of `quaid daemon install` SHALL overwrite the existing unit with the current flag set and reload the service manager. No `uninstall` step is required between reruns. If the flags differ from the previously-installed unit, the new unit SHALL take effect immediately after reload.

#### Scenario: Rerun with new flags replaces the unit and the daemon now hosts SSE
- **WHEN** `quaid daemon install` was previously run without flags, and is now invoked as `quaid daemon install --http --port 3112 --token-file /home/u/.quaid/http_token`
- **THEN** the existing unit is overwritten with the new `ExecStart`/`ProgramArguments` carrying the HTTP flags
- **AND** the service manager is reloaded (`launchctl bootout` + `bootstrap` on macOS; `systemctl daemon-reload` + `restart` on Linux)
- **AND** the running daemon now exposes HTTP/SSE on `127.0.0.1:3112` in-process (no separate transport service is installed)

#### Scenario: Rerun with identical flags is a no-op aside from reload
- **WHEN** `quaid daemon install` is invoked twice in succession with identical flags
- **THEN** both invocations succeed
- **AND** the second invocation does not error on "already installed"

### Requirement: `quaid daemon uninstall` removes the unit and stops the service

`quaid daemon uninstall` SHALL stop the running service via the platform's native control surface and remove the generated unit file. On macOS: `launchctl bootout gui/$UID <plist>` followed by deletion of the plist file. On Linux: `systemctl --user disable --now quaid-daemon` followed by deletion of the unit file and `systemctl --user daemon-reload`. The command SHALL exit successfully even if the unit was not installed (idempotent uninstall). Log files under `~/Library/Logs/quaid-daemon.*` (macOS) SHALL be left in place by default; uninstall is not a log-cleanup operation.

#### Scenario: Uninstall removes plist and stops launchd job (macOS)
- **WHEN** the daemon is installed and `quaid daemon uninstall` is invoked on macOS
- **THEN** `launchctl bootout` is run
- **AND** the plist file is deleted
- **AND** subsequent `launchctl list app.quaid.daemon` returns "could not find service"
- **AND** the log files under `~/Library/Logs/quaid-daemon.*` remain intact

#### Scenario: Uninstall is idempotent when nothing is installed
- **WHEN** `quaid daemon uninstall` is invoked but no unit is installed
- **THEN** the command exits with status 0 and an informational message

### Requirement: `quaid daemon start | stop | restart` controls a running unit

`quaid daemon start`, `quaid daemon stop`, and `quaid daemon restart` SHALL invoke the platform's native control verbs against the installed unit. `start` SHALL fail if the unit is not installed (the user is directed to run `quaid daemon install` first). `stop` SHALL send SIGTERM via the platform manager and wait up to 30 seconds for clean exit before returning a warning; the daemon's own shutdown contract (job-granular, per the `daemon-runtime` capability) determines how long the wait actually takes. `restart` SHALL be equivalent to `stop` followed by `start`.

#### Scenario: Start with no installed unit fails actionably
- **WHEN** `quaid daemon start` is invoked but no unit is installed
- **THEN** the command exits non-zero with a message directing the user to `quaid daemon install`

#### Scenario: Stop waits for graceful job-granular shutdown
- **WHEN** `quaid daemon stop` is invoked while the daemon is mid-extraction-job
- **THEN** SIGTERM is delivered via the platform manager
- **AND** the command waits up to 30 seconds for the daemon to exit cleanly
- **AND** on clean exit the command returns 0

### Requirement: `quaid daemon status` reports installed, running, PID, last activity, and HTTP posture

`quaid daemon status` SHALL report whether the daemon unit is installed, whether it is currently running, the daemon's PID (if running), the absolute database path it is attached to, the timestamp of its last extraction-queue activity, the timestamp of its last vault-sync heartbeat, and — when HTTP is configured — the bind, port, auth state (token-required vs trusted-loopback), and the path to the token file. The output SHALL be human-readable by default and machine-readable with `--json`. Exit codes: `0` if running, `1` if installed-but-stopped, `2` if not installed, `3` on unexpected error reading status.

#### Scenario: Status while daemon is running with HTTP
- **WHEN** the daemon is installed with `--http` and running, and `quaid daemon status` is invoked
- **THEN** the command prints the daemon's PID, database path, last extraction activity, last heartbeat, HTTP bind+port, and auth state (e.g., "token-required" or "trusted-loopback")
- **AND** the command exits with status 0

#### Scenario: Status while daemon is installed but stopped
- **WHEN** the unit is installed but not running and `quaid daemon status` is invoked
- **THEN** the command reports "installed; stopped"
- **AND** the command exits with status 1

#### Scenario: Status when nothing is installed
- **WHEN** no unit is installed and `quaid daemon status` is invoked
- **THEN** the command reports "not installed" with the install command as a suggestion
- **AND** the command exits with status 2

### Requirement: `quaid daemon logs` tails launchd `StandardErrorPath` on macOS and `journalctl` on Linux

`quaid daemon logs` SHALL surface the daemon's stdout/stderr without requiring an `os_log` subsystem rewrite. On macOS the command SHALL read `~/Library/Logs/quaid-daemon.err.log` (and optionally `quaid-daemon.out.log` with `--all-streams`); `--follow` SHALL switch to `tail -F` on that path. On Linux the command SHALL run `journalctl --user -u quaid-daemon -n 200 --no-pager` by default, with `--follow` adding `-f`. The command SHALL exit non-zero if the unit is not installed.

#### Scenario: macOS logs reads the launchd-captured stderr file
- **WHEN** `quaid daemon logs` is invoked on macOS with the daemon installed
- **THEN** the command reads `~/Library/Logs/quaid-daemon.err.log`
- **AND** the output is written to the user's stdout
- **AND** no `log show` or `log stream` subprocess is invoked (no dependency on `os_log` subsystem registration)

#### Scenario: Logs --follow streams in real time
- **WHEN** `quaid daemon logs --follow` is invoked on macOS
- **THEN** the command spawns `tail -F ~/Library/Logs/quaid-daemon.err.log`
- **AND** lines emitted by the daemon thereafter appear in the user's terminal until interrupted

#### Scenario: Linux logs uses journalctl
- **WHEN** `quaid daemon logs` is invoked on Linux
- **THEN** the command spawns `journalctl --user -u quaid-daemon -n 200 --no-pager`
- **AND** `--follow` adds `-f`

### Requirement: `quaid status` is a top-level command distinct from `quaid stats`

The system SHALL provide a top-level `quaid status` command (separate from the existing `quaid stats`) that reports process-level state: daemon installed, daemon running, daemon PID, runtime-host session_type (`daemon` / `serve_host` / none), configured MCP transports (stdio always available; HTTP enabled? port? bind? auth state? `trusted_loopback`?), database path, schema version, last extraction-queue activity, last vault-sync heartbeat. The output SHALL be human-readable by default and machine-readable with `--json`. `quaid stats` (content-level statistics) SHALL remain unchanged.

#### Scenario: Status surfaces daemon, runtime-host, and transport state in one view
- **WHEN** `quaid status` is invoked
- **THEN** the output lists daemon state, current runtime-host session_type, MCP transports, database path, and recent activity in one section per concern
- **AND** the existing `quaid stats` command output is unaffected

#### Scenario: Status --json emits a single object
- **WHEN** `quaid status --json` is invoked
- **THEN** stdout receives a single JSON object with keys `daemon`, `runtime_host`, `transports`, `database`, `activity`
- **AND** the schema is stable enough to script against
