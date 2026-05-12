## Context

`quaid serve` today is a single process that owns a wide collection of concerns at once:

1. **MCP transport** — stdio JSON-RPC 2.0 to a single client.
2. **Vault-sync supervisor + watchers** — file watchers, ownership reconciliation, ingestion pipelines.
3. **Extraction worker** — drains the `extraction_queue`, runs Phi-3.5 via candle, writes fact pages.
4. **A long tail of supervised duties** — idle-close, janitor, quarantine sweep, full-hash audit, RCRT (restore/remap continuation), embedding drain.

The lifetime of all of them is bound to a single `tokio` task: `_service.waiting().await` in `src/mcp/server.rs:511`. When the stdio client closes stdin (the agent exits, the IDE restarts, the terminal closes), that future returns, `main` exits, and every background duty goes with it.

Two issues filed against this:

- **#177** — extraction worker stops processing the queue the moment the client disconnects. Workaround is a cron-driven `quaid extract --all`, which loses the debounce/idle-close semantics and adds latency proportional to the cron interval.
- **#175** — agents that can't speak stdio (REST-only tools, multi-machine setups) can't reach Quaid at all.

The current `daemon-install` change stub (`openspec/changes/daemon-install/proposal.md`) tries to wrap the problem by generating launchd/systemd units around `quaid serve`. That doesn't work: launchd routes stdio to `/dev/null`, so `quaid serve` under `launchctl` would see EOF on stdin immediately and exit. The stub's premise — that "background install" can be added without changing what runs in the background — is the same shape as Issue #177.

Stakeholders: agent integrators who need Quaid always-on (OpenClaw users — see `docs/quaid-vs-qmd-friction-analysis.md:168`), multi-agent system builders (#175), and users running long-form sessions whose tail turns currently never get extracted because the agent exits before the idle timer can fire.

Constraints shaping this design:

- **Single binary.** Splitting into multiple binaries (`quaid-daemon` + `quaid-cli`) breaks the product promise.
- **SQLite is already multi-process safe.** WAL mode plus the existing session-registry surface in `src/core/vault_sync/session.rs` handle concurrent readers/writers. Coordination between processes is a registry lookup, not a new IPC stack.
- **The existing schema-evolution pattern is additive `ALTER`.** `src/core/db.rs:516-532` already extends `serve_sessions` via `ensure_serve_session_columns` without a schema-version bump. New `session_type` values follow that same pattern.
- **No new top-level dependency.** `rmcp` is already in the tree; `transport-sse-server` is a feature flip.
- **Local-first.** HTTP transport is opt-in. Default binding is loopback. Network exposure is an explicit choice that fails closed without authentication.
- **Threat-model honesty.** "Loopback" is not synonymous with "trusted" on hosts that have SSH port-forwards, devcontainers, WSL, or multi-user shells. The design must let operators opt in to auth-on-loopback.

## Goals / Non-Goals

**Goals:**

- Make the *runtime* (workers + watchers + supervised duties) outlive any single MCP transport disconnect.
- Preserve `quaid serve`'s current single-process behavior at the user-facing surface — running `quaid serve` directly under tmux/Ghostty still works without `quaid daemon install`.
- Provide a first-class `quaid daemon install` path that produces a working launchd plist (macOS) and systemd user unit (Linux), with idempotent reinstall.
- Add opt-in HTTP/SSE MCP transport that closes #175 cleanly, with localhost-default and configurable loopback-auth posture.
- Make daemon/transport state observable via `quaid status` so operators can verify their setup at a glance.
- Coordinate worker ownership between `quaid daemon` and `quaid serve` *and between concurrent `quaid serve` invocations* using the existing session-registry surface — no new IPC channel.
- Allow the installed daemon to host the HTTP/SSE MCP transport directly, so an operator running `quaid daemon install --http` gets the always-on multi-agent surface in one step.

**Non-Goals:**

- Multi-host clustering. One runtime owner per database; multiple competing owners against the same DB is explicitly out of scope (and would be a SQLite-WAL contention problem regardless).
- A long-running HTTP/SSE server as the *only* transport. stdio remains the default and the path most agents use.
- An auth provider (OIDC/OAuth). The HTTP transport ships with a single shared bearer token and a fail-closed default. Multi-tenant auth is not in scope.
- Auto-detection of agent connections. The user picks transport via flags; no magic.
- Sandboxing the daemon (chroot, seccomp, etc.). Out of scope for this change.
- A web UI for monitoring the daemon. `quaid status` plus platform log facilities are the supported observation surface.
- Windows service support (`sc.exe` / Windows SCM). Out of scope; macOS + Linux only in v1. Note in operator docs and let users run `quaid daemon run` under their preferred Windows supervisor (`nssm`, Task Scheduler, etc.).

## Decisions

### Decision 1 — `quaid daemon run` is a new foreground entry point, and MAY host the HTTP/SSE MCP transport directly.

The choice was between three shapes:

- **A. Flag on serve** (`quaid serve --no-mcp`): couples user mental models — "serve" already means "open MCP." A flag that says "but don't actually open MCP" is confusing.
- **B. New subcommand** (`quaid daemon run`): clean separation. The verb matches the platform convention (launchd plists invoke `quaid daemon run`, systemd `ExecStart=quaid daemon run`).
- **C. Always run a daemon implicitly on first MCP connection**: too magic; obscures the lifecycle from operators; makes `quaid status` confusing.

**Chosen: B.** `quaid daemon run` is a foreground process. It does not fork, does not detach, does not write a pidfile of its own — those are launchd's/systemd's job. SIGTERM triggers graceful shutdown: stop accepting new queue claims, finish the **current extraction job** (which may span multiple windows; see Decision 8), unregister the session, exit.

**Transport addendum (resolves earlier internal contradiction).** `quaid daemon run` SHALL NEVER open stdio MCP (no terminal-bound transport on a launchd/systemd-managed process), but it MAY open the HTTP/SSE MCP transport in-process when `--http` is supplied. This is what makes `quaid daemon install --http --port 3112` a single coherent command: the installed daemon directly hosts SSE, no second "transport sidecar" service. The alternative — installing two units (`quaid-daemon` + `quaid-serve-http`) — was rejected as friction without benefit: SSE is a tiny tokio task share-hosted in the same process as the workers, and the same `QuaidServer`/`tool_box!` registry handles both transports.

### Decision 2 — Runtime-host coordination via the session registry with an atomic `serve_host` lease, not a flag.

`src/core/vault_sync/session.rs` already has the session-registry surface. The schema also already has a `session_type` column on `serve_sessions` (`src/core/db.rs:524`), extended via additive `ALTER` — currently only the value `'serve'` is in use. This change widens the accepted value set:

- `daemon` — the long-lived runtime under launchd/systemd. Owns watchers + extraction worker + every supervised duty. Optional HTTP/SSE host when started with `--http`.
- `serve_host` — the no-daemon-fallback runtime owner. Exactly one per database. Promoted from a `serve` registration via an atomic lease claim.
- `serve` — a transport-only MCP session. Used when another process already holds runtime-host. Owns nothing except its MCP transport.
- `cli` — a one-shot command (`quaid get`, `quaid put`, etc.). Already exists; unchanged.

`start_serve_runtime` is decomposed:

- `start_daemon_runtime(db_path, http_config: Option<HttpConfig>)` — registers as `daemon`, refuses to start if another live `daemon` row exists, spawns the full runtime, optionally hosts SSE.
- `try_promote_to_serve_host(conn, session_id) -> bool` — atomic transaction that (a) sweeps stale rows, (b) checks for a live `daemon` or live `serve_host`, (c) if both absent, updates the caller's session_type from `'serve'` to `'serve_host'`. Returns `true` on promotion.
- `start_serve_transport(conn, http_config) -> ServeSession` — registers as `serve`, calls `try_promote_to_serve_host`; if it returned `true`, spawns the full runtime; otherwise opens MCP transport only.
- `start_serve_runtime(db_path)` — thin back-compat wrapper that dispatches through `start_serve_transport`, preserving today's `quaid serve` semantics at the boundary.

The lease check is a single transaction. `try_promote_to_serve_host` uses an `INSERT OR ABORT` on a unique constraint (or a `SELECT ... FOR UPDATE` equivalent via `BEGIN IMMEDIATE`) to make the promotion attempt safe under concurrent `quaid serve` startups. The first claimant wins; subsequent claimants stay `'serve'`.

**Alternatives considered:** a filesystem lockfile (`flock` on `~/.quaid/daemon.lock`), a Unix domain socket presence check, or a pidfile. All three would duplicate state that the session registry already maintains, and would not handle the two-concurrent-`serve` race the registry now resolves.

### Decision 3 — HTTP transport uses rmcp's `transport-sse-server` feature, not a hand-rolled HTTP layer.

`rmcp = "0.1"` is already in `Cargo.toml`. Its `transport-sse-server` feature provides MCP-over-SSE that conforms to the MCP spec's HTTP transport. The change is:

```toml
rmcp = { version = "0.1", features = ["transport-sse-server"] }
```

`mcp/server.rs` factors the current `run(conn)` into:

```rust
pub async fn run_stdio(conn: Connection) -> Result<()> { /* today's body */ }
pub async fn run_http(conn: Connection, config: HttpConfig) -> Result<()> { /* SSE */ }
```

Both share the same `QuaidServer` and `tool_box!` registry; no tool needs to know which transport it's running under. Both `quaid serve --http` and `quaid daemon run --http` call `run_http`.

**Alternatives considered:** Axum/Tower with hand-rolled JSON-RPC routing. Rejected — duplicates what `rmcp` already implements correctly, including SSE framing and connection lifecycle.

### Decision 4 — Localhost-default binding, with explicit `trusted_loopback` posture.

The threat model is more honest than "user accidentally exposes Quaid on a LAN." Loopback is also reachable from:

- SSH port-forwards (`ssh -L 3112:127.0.0.1:3112 user@host`)
- WSL2's host-loopback adapter
- VS Code/JetBrains devcontainers and Codespaces
- Multi-user shells where any local user can connect to `127.0.0.1`

The mitigation is **two configurable layers**:

1. **`daemon.http.bind` defaults to `127.0.0.1`.** No way to accidentally listen on `0.0.0.0`.
2. **`daemon.http.trusted_loopback` defaults to `false`.** With `trusted_loopback = false`, even loopback requires a token file; the daemon refuses to start without one. With `trusted_loopback = true`, loopback is unauthenticated (matches stdio's security profile). Operators who want today's stdio-equivalent loopback behavior opt in by setting `trusted_loopback = true`.
3. **Non-loopback bind without `--token-file` is always a startup error**, regardless of `trusted_loopback`. The CLI exits non-zero with an actionable message naming both the offending bind address and the missing flag.

Token file format: one line, base64-url-encoded ≥32 random bytes, file permissions checked at read time (`0600` on Unix; refuse otherwise). The HTTP transport validates `Authorization: Bearer <token>` against the contents of the file. SIGHUP re-reads the file (so rotation is "edit + `kill -HUP <pid>`" or `quaid daemon restart`); a SIGHUP that produces an invalid token logs the failure and keeps the previously-loaded token active.

**Alternatives considered:** mTLS (overkill for personal-memory tooling), OIDC (out of scope for a single-binary tool), Unix-socket-only-on-localhost (defeats #175's cross-machine use case). The shared-bearer-plus-configurable-loopback approach matches Jupyter's notebook server token pattern with a stricter default.

### Decision 5 — `quaid daemon install` rewrites the unit on every run (idempotent).

The first install writes:

- **macOS:** `~/Library/LaunchAgents/app.quaid.daemon.plist`, then `launchctl bootstrap gui/$UID <plist>`.
- **Linux:** `~/.config/systemd/user/quaid-daemon.service`, then `systemctl --user daemon-reload && systemctl --user enable --now quaid-daemon`.

Reruns regenerate the unit file from current arguments and reload. This is intentional: it means the user can change `--http`/`--port`/`--bind`/`--token-file` and rerun `quaid daemon install` without thinking about whether the unit needs reloading. The plist/unit contains the *current* binary path (resolved via `std::env::current_exe()`) and the absolute database path; if the user moves the binary, they rerun install.

**Alternatives considered:** `quaid daemon install` as one-shot (fails on rerun, requires `uninstall` first). Rejected — friction with no upside; idempotent install is the convention for tools like Ollama, Caddy.

### Decision 6 — `quaid daemon install` does not promote `quaid serve` into the daemon role.

`quaid daemon install` writes a unit that invokes `quaid daemon run` — explicitly, not `quaid serve`. This avoids the launchd-stdio-EOF problem entirely. The two surfaces stay distinct:

- `quaid serve` is for interactive MCP transports (an agent attaching to stdio, or a localhost SSE port for an HTTP-only agent).
- `quaid daemon run` is for the headless runtime under launchd/systemd, optionally hosting SSE in-process.

A user who wants both — daemon-installed extraction + stdio MCP for their IDE agent — runs `quaid daemon install` once and then `quaid serve` from their IDE config. The serve process detects the daemon, skips runtime spawn, runs only the MCP transport.

### Decision 7 — `quaid status` is a new top-level command; `quaid stats` is unchanged.

`quaid stats` already exists and reports content-level statistics. `quaid status` is process-level. Two distinct surfaces is clearer than overloading `stats` with `--processes`. The names match operator intuition (`systemctl status`, `docker ps`).

### Decision 8 — Graceful shutdown on SIGTERM is scoped at *job* granularity, not *window* granularity.

The current extractor (`src/core/conversation/extractor.rs:337-372`) processes all windows for a single job, then advances the conversation file's cursor, then marks the queue row `done`. Cursor and queue-done are a unit; a window mid-job is not an independently committable boundary.

Earlier drafts of this proposal specified "finish the current window" — which is incompatible with the current commit model. **Corrected: SIGTERM finishes the current job** (all windows that belong to the same queue row), persists the cursor and queue-done together, then exits. If SIGTERM arrives between jobs, the worker exits within one poll interval. The shutdown budget is therefore one job's worst-case latency — at p95 < 3 s per window times the max windows-per-job for a given session — plus 5 s of bookkeeping. Documented in the daemon-runtime spec and in the operator guide.

If a future change introduces per-window queue rows (making "finish current window" coherent), the spec can tighten. Today's commit model does not support that promise, and the spec must match the model.

### Decision 9 — Owner map for every background duty.

The daemon-runtime/serve-host split must enumerate every background duty owned by today's supervisor, not just watchers + extraction worker. The owner map below applies to every duty in `start_serve_runtime` and `embedding::run_extraction_worker`:

| Duty                       | Owner when daemon live | Owner when no daemon                | Plain `serve` / `cli` |
|----------------------------|------------------------|-------------------------------------|-----------------------|
| File watchers (fsevents/inotify) | daemon                | the unique `serve_host`             | never                 |
| Extraction worker          | daemon                | `serve_host`                        | never                 |
| Idle-close timer           | daemon                | `serve_host`                        | never                 |
| Janitor (queue + corrections cleanup) | daemon         | `serve_host`                        | never                 |
| Quarantine sweep           | daemon                | `serve_host`                        | never                 |
| Full-hash audit            | daemon                | `serve_host`                        | never                 |
| RCRT (restore/remap continuation) | daemon          | `serve_host`                        | never                 |
| Embedding-queue drain      | daemon                | `serve_host`                        | never                 |
| stdio MCP transport        | never                  | when invoked as `quaid serve`       | always (when invoked as `serve`) |
| HTTP/SSE MCP transport     | when `--http`         | when `quaid serve --http`           | n/a                   |
| `cli` one-shots            | n/a                    | n/a                                 | always                |

Tasks 2.5, 3.1, 5.x, 6.x, 7.x are extended to gate each duty on runtime-host ownership rather than on "is this a `quaid serve` process."

### Decision 11 — v1 HTTP transport implements a strict subset of the policy matrix.

Discovered during implementation: rmcp 0.1.5's `SseServer::serve_with_config` builds its own axum router internally and starts `axum::serve` before returning, with no public hook for injecting `tower::Layer` middleware between the `Router` and the running server. The `App`, `sse_handler`, and `post_event_handler` types referenced inside `serve_with_config` are crate-private, so we cannot replicate `serve_with_config` in user code to add an auth layer. Two options were considered:

- **Upgrade or fork rmcp** to expose a `Router`-returning API or wrap the SSE handlers with a layer that supports `tower::Layer<S>`. Out of scope for v1; tracked as a follow-up.
- **Replace rmcp's SSE handlers with a Quaid-owned axum app** that wires the lower-level `RoleServer` trait directly. Roughly a day of work plus its own test surface; also out of scope for v1.

**Chosen for v1:** ship the subset that rmcp 0.1.5 directly supports without bearer-auth, and fail closed on everything else. The implemented policy:

| bind         | token_file | trusted_loopback | v1 outcome                              |
|--------------|------------|------------------|-----------------------------------------|
| loopback     | None       | true             | Listener opens (unauthenticated)        |
| loopback     | None       | false            | Refused (`LoopbackUntrustedNoToken`)    |
| loopback     | Some(_)    | any              | Refused (`BearerAuthDeferred`)          |
| non-loopback | any        | any              | Refused (`NonLoopbackBindUnsupported`)  |

This honors the spec's "fail closed" requirement by the strongest possible means — no listener is ever opened when the requested mode is unsupported. The follow-up that lifts these restrictions can swap `bind_with_token_guard`'s policy without changing any caller; `HttpConfig` already carries every field the eventual implementation needs.

### Decision 10 — `quaid daemon logs` tails launchd's `StandardErrorPath` file on macOS; uses `journalctl` on Linux.

The earlier draft specified `log show --predicate 'subsystem == "app.quaid.daemon"'` for macOS, but the current runtime uses plain `eprintln!` (see `src/core/vault_sync/mod.rs:2807`); there is no `os_log` integration. Rather than adding `os_log` plumbing to every log site, the launchd plist captures stderr to a known file (`~/Library/Logs/quaid-daemon.err.log`) and `quaid daemon logs` tails that file. `--follow` switches to `tail -F`. Linux uses `journalctl --user -u quaid-daemon` because systemd captures stdout/stderr natively into the journal.

An `os_log` integration is a reasonable future improvement (it gives free filtering, structured logs, and remote log streaming), but it is out of scope for v1 to keep the change tightly bounded.

## Risks / Trade-offs

- **[Two concurrent `serve` processes race the promotion lease]** → Mitigated by Decision 2: `try_promote_to_serve_host` is a single SQLite transaction with `BEGIN IMMEDIATE` (or an `INSERT OR ABORT` with a unique constraint on a "runtime-host" virtual row). Test `tests/daemon_serve_coordination.rs` proves only one of two concurrent starts becomes `serve_host`.

- **[User updates the binary; unit still points at old path]** → `quaid daemon install` resolves `current_exe()` at install time. If the user replaces the binary at the same path (the common Homebrew/Cargo upgrade path), no action needed. If they move it, they rerun `quaid daemon install`. Documented.

- **[Token file readable by other local users]** → Permission check at read time. If the file is not `0600`, the daemon refuses to start with an actionable error.

- **[HTTP transport leaks DB path / namespace in error responses]** → The `rmcp` error model already redacts internals. The HTTP layer is a transport, not an error formatter; it inherits the same `map_anyhow_error` / `map_db_error` paths as stdio.

- **[Daemon hangs on shutdown because the worker is mid-extraction job]** → Bounded by Decision 8: one job's worst-case latency plus 5 s bookkeeping. Documented; the operator guide notes the worst-case for sessions with very long windowed extractions.

- **[`quaid daemon install` overwrites a hand-edited unit]** → Idempotency is the design (Decision 5). Operators who hand-roll the unit can skip `install` and write their own.

- **[Cross-platform unit format drift]** → launchd plists and systemd units are independent generators. Tests verify each format against the platform's parser (`plutil -lint` on macOS; `systemd-analyze verify` on Linux).

- **[Adding the HTTP server bloats the static binary]** → `rmcp`'s `transport-sse-server` feature pulls `hyper`/`tower` which are already pulled by other tokio-ecosystem deps. Target: < 2 MB binary-size impact. If it exceeds 4 MB, drop SSE and revisit the `mcp-http-transport` capability scope.

- **[Old binary running against a DB that contains `daemon` / `serve_host` rows]** → Old binaries filter `session_type = 'serve'`, so they treat `daemon` and `serve_host` rows as non-owners. That's the safe fallback for a partial-rollback window — the old binary's `ensure_no_live_serve_owner` will see "no live serve owner" and allow operations. The risk is a short window where two processes might both believe they hold runtime-host. Mitigation: the new binary's promotion check looks at the *full* runtime-owner set, so a new daemon refuses to start if it sees an old-binary `serve` row holding the watcher; users running a mixed setup are advised to `quaid daemon stop` before downgrading. Documented.

- **[Loopback-on-trusted-host expectation regression]** → Today's stdio model treats localhost as trusted. The new `trusted_loopback = false` default changes that for the HTTP transport. Operators who want the old behavior set `daemon.http.trusted_loopback = true` in config or pass `--trust-loopback` to `quaid daemon install`. The CLI prints a one-line note on first install describing the chosen posture.

- **[`os_log` subsystem appears in `daemon logs` output incorrectly]** → Decision 10 routes `daemon logs` to the launchd `StandardErrorPath` file. The spec and tasks explicitly avoid `log show --predicate '... subsystem ...'`.

## Migration Plan

This is a pre-release tool; the no-auto-migration policy applies. Schema additions are additive `ALTER` only. Concretely:

1. Existing users running `quaid serve` directly: **no action**. Behavior is preserved (the first `serve` becomes `serve_host` automatically).
2. Users who want the new always-on behavior: `quaid daemon install` once. Their existing `quaid serve` invocations now skip runtime spawn automatically and run transport-only.
3. Users who want HTTP transport interactively: `quaid serve --http --port 3112` (token required on loopback unless `trusted_loopback = true`).
4. Users who want HTTP transport always-on: `quaid daemon install --http --port 3112 [--token-file PATH]`.
5. The `daemon-install` stub change is superseded; archive it at the same time this change archives.

Rollback: revert the change; users with installed daemons run `quaid daemon uninstall` (or manually `launchctl bootout` / `systemctl --user disable`). The additive column-value expansion is forward/backward compatible — old binaries treat unknown session types as non-owners, which is the safe fallback.

## Open Questions

- **Should `quaid daemon install --http` default to generating a token file automatically?** With `trusted_loopback = false` as the new default, installing `--http` without `--token-file` now fails closed even on loopback. Auto-generating a token would smooth that path. Lean: yes, generate a `~/.quaid/http_token` (mode 0600) when `--http` is passed without `--token-file`, and print the path to stdout. Confirm with the owner.
- **Should `quaid status` exit non-zero when the daemon isn't installed at all?** Current proposal: 0 = running, 1 = installed-but-stopped, 2 = not installed, 3 = error. Alternative: 0/1/0 (not-installed is fine). Lean: stick with 0/1/2/3; cron-style `quaid status && curl ...` is a valid use case that wants "not installed" to be a failure.
- **Windows support cadence.** Out of scope for v1. `quaid daemon run` itself is platform-agnostic; only `install`/`uninstall`/`logs` gate on platform. Land that asymmetric support so Windows users can wire it into `nssm`/Task Scheduler manually. Lean: yes.
- **Health endpoint over HTTP.** `GET /healthz` returning `{"status":"ok","extraction_queue_depth":N,...}` would be useful for external monitors. Defer to a follow-up.
- **`os_log` integration for macOS.** Promised future work; tracked as a follow-up after this change archives.
