## ADDED Requirements

### Requirement: `quaid daemon run` is a foreground runtime entry point that may host HTTP/SSE but never stdio MCP

The system SHALL provide a `quaid daemon run` subcommand that starts the long-lived runtime — the vault-sync supervisor, the file watchers, the extraction worker, the idle-close timer, the janitor, the quarantine sweep, the full-hash audit, the RCRT (restore/remap continuation), and the embedding-queue drain. `quaid daemon run` SHALL NEVER open a stdio MCP transport. With `--http [--port N] [--bind addr] [--token-file PATH]`, `quaid daemon run` MAY open an HTTP/SSE MCP transport in-process per the `mcp-http-transport` capability; without those flags it opens no MCP transport at all. `quaid daemon run` SHALL run as a foreground process, write logs to stdout/stderr, and remain attached to its controlling terminal or supervising service manager. It SHALL NOT fork, daemonize itself, write a pidfile of its own, or detach from its controller.

#### Scenario: Daemon run starts the full runtime with no MCP transport
- **WHEN** `quaid daemon run` is invoked against an initialized database without `--http`
- **THEN** the vault-sync supervisor, file watchers, extraction worker, idle-close timer, janitor, quarantine sweep, full-hash audit, RCRT, and embedding-queue drain are spawned
- **AND** no stdin is read for MCP messages and no TCP port is opened
- **AND** the process remains running until a termination signal is received

#### Scenario: Daemon run with --http opens SSE in-process
- **WHEN** `quaid daemon run --http --port 3112` is invoked
- **THEN** all background duties listed above are spawned (same as the no-`--http` case)
- **AND** an HTTP/SSE MCP transport is opened on `127.0.0.1:3112` (subject to `mcp-http-transport` auth rules)
- **AND** no stdio MCP transport is opened

#### Scenario: Daemon run does not detach
- **WHEN** `quaid daemon run` is invoked from an interactive shell
- **THEN** the process remains attached to that shell's controlling terminal
- **AND** the process does not background itself; backgrounding is the operator's responsibility

### Requirement: Daemon registers as `daemon` session_type in the session registry

On startup, `quaid daemon run` SHALL register a session in the existing `serve_sessions` registry (per the `vault-sync` capability) with `session_type = 'daemon'`. The session registration SHALL fail if another live `daemon` session is already registered for the same database; the new process SHALL exit non-zero with an actionable error naming the existing daemon's PID. Stale daemon sessions (heartbeat past the existing 15-second liveness threshold) SHALL be swept before the registration attempt, identical to how stale `serve` sessions are swept today.

#### Scenario: Daemon refuses to start if another daemon is live
- **WHEN** a `daemon` session row exists with `heartbeat_at > now() - 15s` and a second `quaid daemon run` is invoked
- **THEN** the second invocation exits non-zero with `DaemonAlreadyRunningError` and a message including the live daemon's PID
- **AND** no second background-duty thread is spawned

#### Scenario: Daemon sweeps a stale daemon session and starts
- **WHEN** a `daemon` session row exists with `heartbeat_at < now() - 15s` (process died via SIGKILL) and `quaid daemon run` is invoked
- **THEN** the stale row is swept in the same transaction as the new claim
- **AND** the new daemon starts normally with the full runtime spawned

### Requirement: SIGTERM triggers graceful shutdown at job granularity

The daemon SHALL install a SIGTERM handler that sets a stop flag observed by every background-duty loop. On receiving SIGTERM, the extraction worker SHALL finish the **current extraction job** (which may span multiple windows; the existing per-job cursor-and-queue-done commit model SHALL be preserved) and then exit the loop. The worker SHALL NOT claim a new job after the stop flag is set. Watchers and other supervised duties SHALL stop accepting new work and drain in-flight work to a safe boundary. The supervisor loop SHALL unregister the `daemon` session row and exit with status `0`. Total shutdown time SHALL be bounded by one extraction job's worst-case latency plus 5 seconds of bookkeeping.

#### Scenario: SIGTERM finishes the in-flight job before exit
- **WHEN** the worker is processing a multi-window job when SIGTERM arrives
- **THEN** the worker completes every window of the current job (cursor advances, queue row marks `done`)
- **AND** the supervisor unregisters the daemon session and exits with status 0
- **AND** total shutdown elapsed time is bounded as specified

#### Scenario: SIGTERM between jobs exits quickly
- **WHEN** SIGTERM arrives while the worker is idling between jobs
- **THEN** the worker breaks the idle sleep promptly
- **AND** the supervisor unregisters the daemon session and exits with status 0 within 1 second

### Requirement: Runtime ownership in the no-daemon fallback is held by a unique `serve_host`

When no live `daemon` session is registered for a database, `quaid serve` SHALL attempt to promote its session from `'serve'` to `'serve_host'` via an atomic transaction (per the `vault-sync` capability). The first `serve` process to successfully claim the runtime-host lease SHALL spawn the full background runtime (every duty in the daemon's owner map: watchers, extraction worker, idle-close, janitor, quarantine sweep, full-hash audit, RCRT, embedding drain). Subsequent `serve` processes that fail the promotion SHALL remain `'serve'` and open only their MCP transport. The contract that exactly one process per database hosts the runtime SHALL hold across all combinations of daemon and serve invocations.

#### Scenario: First bare-serve becomes serve_host; second stays transport-only
- **WHEN** no `daemon` session is registered and two `quaid serve` processes start concurrently
- **THEN** exactly one is promoted to `session_type = 'serve_host'` and spawns watchers + workers + every other supervised duty
- **AND** the other remains `session_type = 'serve'` and opens only its MCP transport
- **AND** neither double-spawns the runtime

#### Scenario: Bare serve with live daemon stays transport-only
- **WHEN** a `daemon` session is live and `quaid serve` is invoked
- **THEN** `quaid serve` registers with `session_type = 'serve'`
- **AND** the promotion to `serve_host` is refused because the daemon already owns the runtime
- **AND** no watcher, extraction-worker, or other background-duty thread is spawned by the serve process
- **AND** the daemon's workers drain the queue produced by the serve process's MCP tool calls

### Requirement: Daemon does not silently fetch SLM weights

The daemon SHALL NOT trigger SLM weight downloads. If `extraction.enabled = true` but the configured model is not present in the local cache, the daemon SHALL log a single warning per startup and treat extraction as runtime-disabled, identical to the existing `slm-runtime` contract. Downloads remain owned by `quaid extraction enable` and `quaid model pull <alias>`. This preserves the airgapped-default product property: an unattended daemon never makes a network call for model weights.

#### Scenario: Daemon with missing weights does not download
- **WHEN** `quaid daemon run` starts, `extraction.enabled = true`, and the configured model's cache directory is missing
- **THEN** the daemon logs `slm_runtime_disabled missing_model=<alias>` and proceeds with extraction runtime-disabled
- **AND** no outbound HTTP request to a model host is made
- **AND** the daemon continues to serve as the watcher owner for vault-sync
