## 1. Session-type expansion (additive, no schema-version bump)

- [ ] 1.1 In `src/core/db.rs:516-532`, keep the existing `ensure_serve_session_columns` additive `ALTER` shim. Do **NOT** bump `SCHEMA_VERSION`. Update the column's accepted set of values from `{'serve'}` to `{'daemon','serve_host','serve','cli'}` at the application layer (the additive `ALTER` already keeps the column as plain `TEXT NOT NULL DEFAULT 'serve'`)
- [ ] 1.2 Add a `SessionType` enum (`Daemon`, `ServeHost`, `Serve`, `Cli`) in `src/core/vault_sync/session.rs` and a `to_db_str(&self) -> &'static str` helper that maps to the four string values
- [ ] 1.3 Extend `register_session(conn, role: SessionType)` to persist the value; update the existing `register_cli_session` to set `SessionType::Cli` explicitly
- [ ] 1.4 Add `find_active_daemon_session(conn) -> Result<Option<SessionInfo>>` and `find_active_runtime_host(conn) -> Result<Option<SessionInfo>>` (returns the live `daemon` OR `serve_host`, whichever exists)
- [ ] 1.5 Unit tests: insert one row of each session_type, sweep across all types behaves identically; old-binary simulation (filter `session_type = 'serve'`) treats `daemon`/`serve_host` rows as non-owners (this is the safe partial-rollback fallback verified by `tests/ownership_session_type_audit.rs`)
- [ ] 1.6 Verify that a v9-or-later DB without the column gains the column via the existing additive ALTER on first open; no schema-version mismatch error is raised

## 2. Atomic `serve_host` promotion

- [ ] 2.1 Implement `try_promote_to_serve_host(conn, session_id) -> Result<bool, VaultSyncError>` in `src/core/vault_sync/session.rs` as a single `BEGIN IMMEDIATE` transaction that (a) sweeps stale rows, (b) checks for a live `daemon` or live `serve_host`, (c) if both absent, UPDATEs the caller's row `session_type` from `'serve'` to `'serve_host'`
- [ ] 2.2 Tests: concurrent calls from two threads — exactly one returns `true`; a `daemon` row blocks promotion; a stale `daemon` row is swept and promotion succeeds
- [ ] 2.3 Tests: promotion is idempotent if the caller is already `'serve_host'` (returns `true` without error)

## 3. Decompose `start_serve_runtime`

- [ ] 3.1 Create `start_daemon_runtime(db_path, http_config: Option<HttpConfig>) -> Result<ServeRuntime, VaultSyncError>` in `src/core/vault_sync/mod.rs`: register with `SessionType::Daemon`; refuse if `find_active_daemon_session` returns `Some`; spawn the full background runtime (watchers, extraction worker, idle-close, janitor, quarantine sweep, full-hash audit, RCRT, embedding drain); when `http_config` is `Some`, also start `mcp::server::run_http`
- [ ] 3.2 Create `start_serve_transport(db_path, http_config: Option<HttpConfig>) -> Result<ServeRuntime, VaultSyncError>`: register with `SessionType::Serve`; call `try_promote_to_serve_host`; if `true`, spawn the full background runtime (same set as `start_daemon_runtime`); regardless, spawn the MCP transport (stdio or HTTP per `http_config`)
- [ ] 3.3 Keep `start_serve_runtime(db_path)` as a thin back-compat wrapper that dispatches to `start_serve_transport(db_path, None)` so today's callers in `src/commands/serve.rs` keep working
- [ ] 3.4 Update `ServeRuntime::Drop` (or the equivalent shutdown path) to handle both `daemon`/`serve_host` (full runtime to tear down) and `serve` (transport only) without double-stops
- [ ] 3.5 Wire SIGTERM (and SIGINT on interactive runs) handler in `start_daemon_runtime` that sets `stop: Arc<AtomicBool>` observed by every background-duty loop
- [ ] 3.6 Tests: `tests/daemon_serve_coordination.rs` verifies (a) daemon spawns full runtime, (b) serve with live daemon does not spawn, (c) serve without daemon promotes and spawns, (d) two concurrent serves yield one `serve_host` + one `serve`, (e) second daemon refuses to start

## 4. Owner-map gating for every supervised duty

- [ ] 4.1 Audit `start_serve_runtime`'s current thread-spawn sites in `src/core/vault_sync/mod.rs` and list every duty: file watchers, extraction worker, idle-close, janitor, quarantine sweep, full-hash audit, RCRT, embedding-queue drain. Add a `RuntimeHostScope` struct (or use a per-duty `Option<JoinHandle>` field on `ServeRuntime`) so each duty is conditionally spawned only when the process is the runtime host
- [ ] 4.2 Move every duty's spawn point into `start_daemon_runtime` and `start_serve_transport`'s promoted branch; leave `start_serve_transport`'s un-promoted branch with **zero** supervised-duty spawns
- [ ] 4.3 Verify by code search (`grep -n 'thread::spawn\|tokio::spawn'` in `src/core/vault_sync/mod.rs`) that every spawn is reachable only from the runtime-host code paths
- [ ] 4.4 Tests: `tests/daemon_runtime.rs` enumerates thread/task counts at steady state in (a) daemon-only mode, (b) serve_host mode, (c) serve-with-daemon mode and asserts the expected counts per the owner map in `design.md` Decision 9

## 5. Extraction worker — job-granular cooperative shutdown

- [ ] 5.1 In `src/core/conversation/extractor.rs`, make the worker poll loop check `stop_flag` only at job boundaries — between `claim_next_job` calls — to align with the per-job cursor-and-queue-done commit model at lines 337-372
- [ ] 5.2 If `stop_flag` is observed mid-job (between windows of the same job), continue processing windows of that job until the existing per-job commit completes, then exit the loop
- [ ] 5.3 Make the idle-sleep interruptible (e.g., `park_timeout` + `unpark` from the SIGTERM handler) so shutdown latency from flag-set with an empty queue is under 2 seconds
- [ ] 5.4 Tests: `tests/daemon_runtime.rs` covers (a) SIGTERM mid-job finishes that job (all its windows) and exits within one job's worst-case latency + 5s bookkeeping, (b) SIGTERM at idle exits ≤ 2s, (c) SIGTERM between jobs does not consume the next pending queue row

## 6. `ownership.rs` audit — widen `session_type = 'serve'` filters

- [ ] 6.1 Update `live_collection_owner` in `src/core/vault_sync/ownership.rs:46-68`: change `WHERE ... AND s.session_type = 'serve'` to `WHERE ... AND s.session_type IN ('daemon','serve_host','serve')`
- [ ] 6.2 Update `live_collection_owner_for_root_path` at `ownership.rs:70-98` with the same filter widening
- [ ] 6.3 Update `ensure_no_live_serve_owner` at `ownership.rs:100-117` — uses `live_collection_owner` so this is downstream-covered; but verify the error path emits a message that names the actual `session_type` rather than implying "a `quaid serve` process"
- [ ] 6.4 Update `ensure_no_live_serve_owner_for_root_path` at `ownership.rs:122-135` similarly
- [ ] 6.5 Update `acquire_owner_lease` at `ownership.rs:156-191` — its `live_collection_owner` call is already covered by 6.1; verify no other inline filter is `'serve'`-only
- [ ] 6.6 Add `session_type` to the `LiveCollectionOwner` struct so consumers can surface the actual role in error messages
- [ ] 6.7 Code search to confirm no other module filters `serve_sessions` by `session_type = 'serve'` alone (`grep -rn "session_type" src/`); document every additional hit and update it or justify why it's left alone
- [ ] 6.8 Tests: `tests/ownership_session_type_audit.rs` inserts fixture rows of each session type and asserts every public predicate in `ownership.rs` recognizes `daemon` and `serve_host` as owners

## 7. Rename `ServeOwnsCollectionError` → `RuntimeOwnsCollectionError`

- [ ] 7.1 Rename the variant in `src/core/vault_sync/mod.rs` (or wherever the `VaultSyncError` enum lives) and update its message template to include `session_type`
- [ ] 7.2 Add a `#[deprecated]` type alias `pub type ServeOwnsCollectionError = RuntimeOwnsCollectionError` if any external module depends on the old name; otherwise just rename
- [ ] 7.3 Update every `match`/`if let` arm referencing `ServeOwnsCollectionError`
- [ ] 7.4 Operator-facing messages for `quaid collection restore` etc. now recommend `quaid daemon stop` when the owner is `daemon`, and `kill <pid>` when the owner is `serve_host`
- [ ] 7.5 Tests: update fixtures that match on the old error name; add a test that the new message includes `session_type` and the right stop recommendation

## 8. `quaid daemon run` foreground entry point

- [ ] 8.1 Create `src/commands/daemon.rs` with `DaemonAction` enum (`Run`, `Install`, `Uninstall`, `Start`, `Stop`, `Restart`, `Status`, `Logs`)
- [ ] 8.2 Implement `DaemonAction::Run`: parse `--http`/`--port`/`--bind`/`--token-file`/`--trust-loopback` flags into `Option<HttpConfig>`; call `start_daemon_runtime(db_path, http_config)`; log `daemon ready pid=<pid> db=<path> http=<port-or-none>` once registered; block on `_runtime.handle.join()`
- [ ] 8.3 On `DaemonAlreadyRunningError`, exit non-zero with a message including the live daemon's PID and host
- [ ] 8.4 Wire `Commands::Daemon { action }` in `src/main.rs` clap subcommand surface
- [ ] 8.5 Test: invoke `quaid daemon run` in a subprocess; assert PID and HTTP state are logged; send SIGTERM; assert clean exit and the daemon row is gone from `serve_sessions`

## 9. Platform-native unit generators

- [ ] 9.1 Create `src/platform/mod.rs` exposing `Platform` enum and `generate_unit(args: &UnitArgs) -> Result<UnitFile>` dispatch
- [ ] 9.2 `src/platform/launchd.rs` (`#[cfg(target_os = "macos")]`): generate `app.quaid.daemon.plist` with `Label`, `ProgramArguments` (absolute binary path + `daemon run` + optional `--http`/`--port`/`--bind`/`--token-file`/`--trust-loopback`), `RunAtLoad`, `KeepAlive` (with `SuccessfulExit = false`), `StandardOutPath = ~/Library/Logs/quaid-daemon.out.log`, `StandardErrorPath = ~/Library/Logs/quaid-daemon.err.log`, `EnvironmentVariables` (carrying `QUAID_DB_PATH`). Ensure the `Logs/` directory exists at install time
- [ ] 9.3 `src/platform/systemd.rs` (`#[cfg(target_os = "linux")]`): generate `quaid-daemon.service` with `[Unit]`, `[Service]` (`Type=simple`, `ExecStart=<binary> daemon run [flags]`, `Restart=on-failure`, `Environment=QUAID_DB_PATH=...`), `[Install]` `WantedBy=default.target`
- [ ] 9.4 Helpers: `launchctl_bootstrap`, `launchctl_bootout`, `launchctl_kickstart`, `systemctl_user_*` that shell out and surface non-zero exits as typed errors
- [ ] 9.5 Tests: golden-file tests for plist and unit content with deterministic inputs; runtime tests skip on non-host platforms

## 10. `quaid daemon install | uninstall | start | stop | restart`

- [ ] 10.1 `Install`: resolve `current_exe()`, ensure `~/Library/Logs/` exists (macOS), generate unit, write to platform path, run `bootstrap`/`enable --now`. Idempotent: if file exists, overwrite and `bootout`+`bootstrap` (mac) or `daemon-reload` + `restart` (linux). When `--http` is passed without `--token-file`, auto-generate `~/.quaid/http_token` (mode 0600) and reference it in the unit (per the open question resolution in `design.md`)
- [ ] 10.2 `Uninstall`: run `bootout`/`disable --now`, delete unit file, run `daemon-reload` (linux). Exit 0 even when nothing was installed. Do NOT delete log files
- [ ] 10.3 `Start`: require unit installed; run `kickstart -k` (mac) or `start` (linux)
- [ ] 10.4 `Stop`: deliver SIGTERM via `bootout` (mac) or `stop` (linux); wait up to 30 s for clean exit; warn if timeout
- [ ] 10.5 `Restart`: equivalent to `stop` + `start`
- [ ] 10.6 Tests: `tests/daemon_lifecycle_cli.rs` covers install→start→status→stop→uninstall round-trip on host platform; idempotent reinstall with changed flags; install on Windows/other exits 2 with documented message; install --http auto-generates the token file at the documented path

## 11. `quaid daemon status` and `quaid daemon logs`

- [ ] 11.1 `Status`: probe unit installed (file exists at platform path); probe running (parse `launchctl print` or `systemctl --user is-active`); read daemon PID, db path, last queue activity (`SELECT MAX(updated_at) FROM extraction_queue`), last vault-sync heartbeat (`SELECT heartbeat_at FROM serve_sessions WHERE session_type = 'daemon'`); when HTTP is configured, include bind, port, auth state (`token-required` / `trusted-loopback`), token file path
- [ ] 11.2 Status exit codes: 0 running, 1 installed-but-stopped, 2 not installed, 3 error
- [ ] 11.3 `--json` flag emits a stable schema (`{installed: bool, running: bool, pid: u32?, db_path: String, last_extraction_at: String?, last_heartbeat_at: String?, http: {enabled: bool, bind: String?, port: u16?, auth: "token-required" | "trusted-loopback"?, token_file: String?}?}`)
- [ ] 11.4 `Logs`: on macOS, read `~/Library/Logs/quaid-daemon.err.log` (and `quaid-daemon.out.log` with `--all-streams`); `--follow` runs `tail -F` on the chosen file. On Linux, spawn `journalctl --user -u quaid-daemon -n 200 --no-pager` (or add `-f` with `--follow`). Do **NOT** invoke `log show --predicate 'subsystem == ...'` on macOS — the daemon writes to stdout/stderr only and the launchd unit captures those streams to the log files
- [ ] 11.5 Tests: status command output shape (human + JSON); status exit codes for each of the four states; logs reads the launchd file on macOS (use a temp `HOME` so the test can write a fixture log) and invokes `journalctl` with the right args on Linux

## 12. Top-level `quaid status` command (distinct from `quaid stats`)

- [ ] 12.1 Create `src/commands/status.rs` with a `run(db, json: bool)` entry point that aggregates daemon state, runtime-host `session_type`, configured MCP transports, db path/schema version, last activity timestamps
- [ ] 12.2 Add `Commands::Status { json: bool }` in `src/main.rs`
- [ ] 12.3 Verify `quaid stats` output is unchanged (regression test on its output golden)
- [ ] 12.4 Tests: `tests/status_command.rs` verifies sections present in human mode and keys present in `--json` mode

## 13. MCP HTTP/SSE transport — Cargo and runtime

- [ ] 13.1 `Cargo.toml`: change `rmcp = "0.1"` to `rmcp = { version = "0.1", features = ["transport-sse-server"] }`; run `cargo build` and verify no new top-level dep
- [ ] 13.2 In `src/mcp/server.rs`, factor `run(conn)` into `run_stdio(conn)` (today's body) and `run_http(conn, config: HttpConfig)` (new); both share `QuaidServer::new(conn)` and the `tool_box!` registry
- [ ] 13.3 Define `HttpConfig { port: u16, bind: IpAddr, token_file: Option<PathBuf>, trusted_loopback: bool }` with defaults `(3112, 127.0.0.1, None, false)`
- [ ] 13.4 Implement `bind_with_token_guard(config) -> Result<Listener>`:
    - non-loopback bind without `token_file` → reject regardless of `trusted_loopback`
    - loopback bind without `token_file` and `trusted_loopback = false` → reject
    - loopback bind without `token_file` and `trusted_loopback = true` → accept, no auth
    - any bind with `token_file` → validate file (mode 0600, single line, decoded ≥32 bytes); enforce bearer auth
- [ ] 13.5 Tests: every combination of `(bind in {loopback, 0.0.0.0}) × (token_file in {Some, None}) × (trusted_loopback in {true, false})` covered with the expected accept/reject outcome

## 14. HTTP transport — `quaid serve --http` CLI surface

- [ ] 14.1 Extend `Commands::Serve` in `src/main.rs` to accept `--http`, `--port <N>`, `--bind <addr>`, `--token-file <path>`, `--trust-loopback` flags
- [ ] 14.2 Update `src/commands/serve.rs` so `--http` calls `mcp::server::run_http(conn, config)` and stdio remains the default; `--port`/`--bind`/`--token-file`/`--trust-loopback` SHALL only be accepted when `--http` is also passed (clap-level validation)
- [ ] 14.3 Tests: `tests/mcp_http_transport.rs` covers (a) loopback with `--trust-loopback` accepts unauthenticated tool call, (b) loopback without `--trust-loopback` and no `--token-file` exits non-zero before listening, (c) loopback with `--token-file` enforces bearer auth, (d) `--bind 0.0.0.0` without `--token-file` exits non-zero even with `--trust-loopback`, (e) `--bind 0.0.0.0` with `--token-file` enforces bearer auth, (f) bad bearer returns 401

## 15. Token file lifecycle and SIGHUP reload

- [ ] 15.1 Implement `TokenStore::load(path: &Path) -> Result<TokenStore>` with mode + entropy + single-line validation; store decoded bytes inside an `Arc<RwLock<Bytes>>` for SIGHUP swap
- [ ] 15.2 Wire SIGHUP handler in both `start_daemon_runtime` and `start_serve_transport` that re-reads the token file via `TokenStore::reload`; on validation failure, log error and keep the previously-loaded token
- [ ] 15.3 Tests: SIGHUP with valid new token activates it for subsequent connections; SIGHUP with invalid new token leaves the old one active and logs `token_reload_failed`
- [ ] 15.4 Verify token contents never appear in any error message or log line (negative test: build error path with known-good token, assert error string does not contain the token bytes)

## 16. Config keys and defaults

- [ ] 16.1 Add config keys to seed defaults in `src/schema.sql`: `daemon.http.enabled='false'`, `daemon.http.port='3112'`, `daemon.http.bind='127.0.0.1'`, `daemon.http.token_file` (NULL by default), `daemon.http.trusted_loopback='false'`
- [ ] 16.2 `quaid daemon install` flags override config defaults at install time and are baked into the unit's argv
- [ ] 16.3 Tests: config defaults present in fresh DB; install flags overwrite into the unit; reinstall with different flags overwrites again; the `trusted_loopback` config default is `false` (new behavior — breaks the "loopback == trusted" assumption deliberately)

## 17. Operator docs

- [ ] 17.1 Write `docs/operator-guide-daemon.md`: install/uninstall, what gets written where, how to confirm it's running, how to inspect logs (note: tail of launchd-captured stderr on macOS, journalctl on Linux), how to enable HTTP, how to mint a token file (`openssl rand -base64 32 > ~/.quaid/http_token && chmod 600 ~/.quaid/http_token`), `trusted_loopback` guidance (when to opt in, when not to — especially SSH-forward / WSL / devcontainer caveats), troubleshooting matrix (binary moved, token file deleted, port in use, mixed-binary partial-rollback)
- [ ] 17.2 Update `docs/quaid-vs-qmd-friction-analysis.md` notes (or add a follow-up note) calling out that the daemon-friction gap is closed
- [ ] 17.3 Update `README.md` (or relevant top-level doc) Quick Start to mention `quaid daemon install` as the recommended setup after first `quaid collection add`

## 18. Supersede the `daemon-install` stub

- [ ] 18.1 Add a one-line note in `openspec/changes/daemon-install/proposal.md` stating that this change is superseded by `daemon-and-http-transport` and should be archived together with it
- [ ] 18.2 Decide with the owner whether to delete `openspec/changes/daemon-install/` at archive time of `daemon-and-http-transport`, or to fold the stub's archive into this one's archive PR

## 19. Pre-merge verification

- [ ] 19.1 `cargo fmt --all -- --check` passes (project pre-push gate; CI runs the same check)
- [ ] 19.2 `cargo clippy --all-targets -- -D warnings` passes
- [ ] 19.3 `cargo test --all-targets` passes on macOS and Linux runners
- [ ] 19.4 Manual smoke: `quaid daemon install` on macOS and Linux; verify launchctl/systemctl status; run `quaid serve` against the live daemon and confirm the second `serve` does not double-spawn (transport-only registration in `serve_sessions`); send SIGTERM and confirm the current extraction job completes before the daemon exits
- [ ] 19.5 Binary-size budget: `cargo build --release` artifact size grows by < 2 MB vs pre-change baseline; fail the review if > 4 MB
- [ ] 19.6 Mixed-binary check: spin up an older binary holding an old-style `session_type = 'serve'` `collection_owners` row, then call the new binary's `ensure_no_live_serve_owner` — it MUST return `RuntimeOwnsCollectionError`, proving the partial-rollback safety net works
