# Daemon Operator Guide

Quaid's background runtime (vault sync, file watchers, extraction worker, idle-close, janitor) can run in three shapes:

1. **Bare `quaid serve` under tmux/Ghostty/your terminal.** The first invocation auto-promotes to runtime owner via the `serve_host` lease; subsequent invocations against the same database stay transport-only. This is the original shape and is what you get with no extra setup.
2. **Installed daemon + interactive `quaid serve`.** A long-lived `quaid daemon run` process owns the runtime under launchd/systemd; `quaid serve` invocations from IDEs/agents attach as transport-only MCP clients. This is the recommended setup for daily use.
3. **Installed daemon with HTTP/SSE MCP transport.** Same as (2) but the daemon also exposes an MCP-over-HTTP listener so REST-only agents and multi-machine setups can reach it. Loopback-only in v1; see [HTTP transport limitations](#http-transport-limitations).

---

## Install

```bash
# macOS or Linux — installs the platform service unit and starts it.
quaid daemon install
```

This writes:

- **macOS:** `~/Library/LaunchAgents/app.quaid.daemon.plist` and bootstraps it via `launchctl bootstrap gui/<uid>`. Logs go to `~/Library/Logs/quaid-daemon.{out,err}.log`.
- **Linux:** `~/.config/systemd/user/quaid-daemon.service` and starts it via `systemctl --user enable --now quaid-daemon`. Logs go to the systemd user journal.

The generated unit invokes whatever `quaid` binary `current_exe()` returns at install time. **If you move or replace the binary, rerun `quaid daemon install`** to refresh the unit's `ExecStart`/`ProgramArguments`.

`install` is idempotent: rerunning it rewrites the unit and reloads the service manager. No `uninstall` step is needed between reruns.

### Install with HTTP transport

```bash
quaid daemon install --http --port 3112 --trust-loopback
```

This bakes `--http --port 3112 --bind 127.0.0.1 --trust-loopback` into the daemon's argv. The daemon will open an SSE listener on `127.0.0.1:3112` in addition to running the background runtime. See [HTTP transport limitations](#http-transport-limitations) for what's supported in v1.

---

## Confirm it's running

```bash
quaid daemon status
# or
quaid status            # process-level overview (daemon + transports + activity)
quaid status --json     # machine-readable
```

Exit codes for `quaid daemon status` and `quaid status`:

| code | meaning                                  |
|------|------------------------------------------|
| `0`  | daemon running                            |
| `1`  | daemon installed but stopped              |
| `2`  | daemon not installed                      |
| `3`  | unexpected error reading status           |

---

## Inspect logs

```bash
quaid daemon logs               # last ~200 lines
quaid daemon logs --follow      # stream new lines
quaid daemon logs --all-streams # macOS: also include stdout (no-op on Linux)
```

Implementation detail: on macOS this is a `tail -F ~/Library/Logs/quaid-daemon.err.log` (the launchd plist captures stderr to a known file). On Linux it's `journalctl --user -u quaid-daemon`. There is no `os_log` subsystem registration — the daemon writes to plain stdout/stderr and the service manager captures both streams.

---

## Uninstall

```bash
quaid daemon uninstall
```

Stops the service via `launchctl bootout` (macOS) / `systemctl --user disable --now` (Linux), then removes the unit file. **Log files are preserved** so you can keep a historical record after uninstalling. Uninstall is idempotent and exits `0` even if nothing was installed.

---

## HTTP transport

### Limitations

v1 supports only the simplest HTTP transport mode: **loopback bind under `--trust-loopback`, unauthenticated**. The full security policy described in the `mcp-http-transport` capability is the long-term target; what's deferred:

- Bearer-token authentication (`--token-file`) is parsed by the CLI but **not enforced** in v1. The startup guard rejects any configuration that supplies `--token-file` to avoid giving operators a false sense of security.
- Non-loopback bind (`--bind 0.0.0.0` or any non-loopback address) is **always rejected at startup** in v1, regardless of `--token-file` / `--trust-loopback`. No TCP listener is ever opened on a non-loopback port.
- SIGHUP token reload is deferred along with bearer-auth.

These limitations exist because the underlying `rmcp` 0.1.5 SSE server doesn't expose middleware hooks for bearer-auth. They're tracked as a follow-up to lift once the supporting plumbing is in place.

### Enable on the daemon

```bash
quaid daemon install --http --port 3112 --trust-loopback
quaid daemon status        # expect "daemon: running"
curl -s http://127.0.0.1:3112/sse
# (the SSE stream is for MCP clients, not human-readable; just confirms the listener is up)
```

### Enable on interactive serve

```bash
quaid serve --http --port 3112 --trust-loopback
```

`--http`, `--port`, `--bind`, `--token-file`, `--trust-loopback` all require `--http` to be set; clap rejects any combination that supplies sub-flags without it.

### `trusted_loopback` guidance

`daemon.http.trusted_loopback = false` is the default (refuse loopback without auth) because **loopback is not synonymous with "trusted"** on hosts that have any of:

- SSH port-forwarding (`ssh -L 3112:127.0.0.1:3112 user@host` exposes your local loopback to a remote user)
- WSL2's host-loopback adapter
- VS Code/JetBrains devcontainers, Codespaces
- Multi-user shells where any local user can reach `127.0.0.1`

`--trust-loopback` opts you in to the stdio-equivalent security profile (any local process can connect, no auth). Only do this on a trusted single-user host without remote port-forwarding.

---

## Coordination with `quaid serve`

You can run `quaid serve` while a daemon is installed; it will detect the live `daemon` session and register itself as a transport-only `serve` session — no watchers, no extraction worker, no double-spawn. The daemon owns the runtime; `serve` is just the MCP transport for your IDE/agent.

If you stop the daemon (`quaid daemon stop`) while `quaid serve` is running, the next `quaid serve` invocation after the daemon's session-row sweep window (~15s) will auto-promote to `serve_host` and take over runtime ownership.

Foreground runtimes (`quaid serve` and `quaid daemon run`) handle SIGTERM/SIGINT by stopping owned workers and unregistering their `serve_sessions` row. Use the platform service manager (`quaid daemon stop`) for installed daemons; use a direct signal only for foreground/manual processes.

---

## Troubleshooting

### Binary moved or upgraded
The unit's `ExecStart`/`ProgramArguments` points at the binary path that was active at install time. After moving or replacing the binary at a different path:

```bash
quaid daemon install        # rewrites the unit with the new current_exe()
```

If the binary stays at the same path (the common Homebrew/Cargo upgrade case), no action is needed.

### Port already in use
If port 3112 is already taken (another Quaid daemon, an unrelated service):

```bash
quaid daemon install --http --port 3113 --trust-loopback
```

### Daemon won't start: "DaemonAlreadyRunningError"
Another live `daemon` session is registered for this database. Find it:

```bash
quaid status
# Reports the live daemon's PID and host. Stop it via:
quaid daemon stop
# or, if it's not under launchd/systemd:
kill <pid>
```

### "RuntimeOwnsCollectionError" on `quaid collection restore` / `migrate-uuids`
The runtime (daemon or serve_host) holds the collection's owner lease. Stop the runtime first:

- If `quaid status` reports `session_type=daemon` → `quaid daemon stop`
- If `quaid status` reports `session_type=serve_host` → `kill <pid>` (or close the terminal running `quaid serve`)

Then rerun the command. The error message names the actual session_type and the right stop verb.

### Mixed-binary partial rollback
An older binary (one that predates the `serve_sessions.session_type` widening) reading the same database will filter `WHERE session_type = 'serve'` and treat `daemon` / `serve_host` rows as non-owners. That's the safe fallback — restore/remap commands continue to refuse mutation when the runtime is live — but it also means the older binary cannot see the daemon for status purposes. If you're partially rolled back, stop the new-binary daemon (`quaid daemon stop`) before running ops with the older binary.

### Logs are empty
`quaid daemon logs` requires the daemon to have written at least one line since install. Run `quaid daemon stop && quaid daemon start` to bounce it and force a `daemon_ready` line into the log.

### `quaid daemon install` fails on Windows
Windows isn't supported in v1. You can still run `quaid daemon run` directly under your preferred supervisor (`nssm`, Task Scheduler, etc.) — the foreground entry point itself is platform-agnostic.
