## ADDED Requirements

### Requirement: v1 implementation scope and known limitations
The system SHALL implement the HTTP/SSE transport on top of `rmcp` 0.1.5's `SseServer` API, which does not expose middleware hooks for bearer-auth. v1 SHALL therefore implement only the subset of the policy below that rmcp 0.1.5 directly supports without forking or replacing its SSE handlers: loopback bind under `--trust-loopback` (unauthenticated, stdio-equivalent). Every other combination (loopback without `--trust-loopback`, loopback with `--token-file`, any non-loopback bind) SHALL fail closed at startup with a typed error before any TCP listener is opened. This satisfies the "fail closed" requirement below by the strongest possible means (no listener) and tracks bearer-auth enforcement, non-loopback bind, and SIGHUP token reload as follow-ups. The `HttpConfig` struct SHALL carry every field the eventual full policy needs (`port`, `bind`, `token_file`, `trusted_loopback`) so the deferred work is purely additive in `bind_with_token_guard` rather than a wider refactor.

#### Scenario: v1 build refuses loopback-with-token
- **WHEN** `quaid serve --http --token-file /path/to/token` is invoked in a v1 binary
- **THEN** the process exits non-zero with a `BearerAuthDeferred` error message naming the v1 limitation
- **AND** no TCP listener is opened on the configured port

#### Scenario: v1 build refuses non-loopback bind
- **WHEN** `quaid serve --http --bind 0.0.0.0` is invoked in a v1 binary regardless of `--token-file` / `--trust-loopback` combination
- **THEN** the process exits non-zero with a `NonLoopbackBindUnsupported` error
- **AND** no TCP listener is opened on the configured port

### Requirement: `quaid serve --http` and `quaid daemon run --http` both open the MCP SSE transport

The system SHALL provide an opt-in HTTP/SSE MCP transport via both `quaid serve --http [...]` (interactive) and `quaid daemon run --http [...]` (under launchd/systemd, per the `daemon-runtime` capability). The transport SHALL be backed by the `rmcp` crate's `transport-sse-server` feature, exposing the same MCP tool registry as the stdio transport. When `--http` is passed to `quaid serve`, stdio MCP SHALL NOT be opened on the same invocation; stdio and HTTP transports are mutually exclusive per `serve` invocation. `quaid daemon run` SHALL NEVER open stdio MCP; the `--http` flag is the only way to expose an MCP transport from a daemon.

#### Scenario: `quaid serve --http` opens SSE on the configured port
- **WHEN** `quaid serve --http --port 3112` is invoked with `trusted_loopback = true`
- **THEN** the process listens on `127.0.0.1:3112` and accepts MCP SSE connections
- **AND** stdin is not consumed as an MCP transport

#### Scenario: `quaid daemon run --http` hosts SSE in the daemon process
- **WHEN** `quaid daemon run --http --port 3112 --token-file /home/u/.quaid/http_token` is invoked
- **THEN** the daemon process spawns the full background runtime AND listens on `127.0.0.1:3112` for SSE
- **AND** no second transport sidecar process is required
- **AND** authenticated requests succeed; unauthenticated requests are rejected

#### Scenario: MCP tools work identically over HTTP
- **WHEN** an MCP client connects to either `quaid serve --http` or `quaid daemon run --http` and invokes `memory_search` with valid arguments
- **THEN** the response matches what the stdio transport would return for the same arguments
- **AND** no tool is excluded from the HTTP surface

### Requirement: HTTP transport defaults to loopback binding

When `--bind` is omitted, the HTTP transport SHALL bind to `127.0.0.1` only. The default port SHALL be `3112`. The TCP layer SHALL reject connections from non-loopback peers regardless of authentication state.

#### Scenario: Default bind is loopback
- **WHEN** `quaid serve --http` is invoked with no `--bind`
- **THEN** the listener is bound to `127.0.0.1:3112`
- **AND** the process refuses to accept connections from non-loopback peers (TCP layer rejects them)

### Requirement: Loopback bind requires a token unless `trusted_loopback = true`

The system SHALL gate unauthenticated loopback access behind a `daemon.http.trusted_loopback` config key (default `false`) and a matching `--trust-loopback` CLI flag. With `trusted_loopback = false` (the default), loopback SHALL require `--token-file` and SHALL fail closed at startup if no token file is configured. With `trusted_loopback = true`, loopback MAY be opened without a token (matching stdio's security profile and the previous "localhost is trusted" assumption). This addresses the SSH-port-forward, WSL, devcontainer, and shared-user-host threat surfaces where loopback is not equivalent to "physically local trusted access."

#### Scenario: Loopback without token under default trust posture refuses to start
- **WHEN** `quaid serve --http --port 3112` is invoked with `daemon.http.trusted_loopback = false` and no `--token-file`
- **THEN** the process exits with status non-zero
- **AND** the error message names the missing `--token-file` flag and suggests either supplying one or setting `trusted_loopback = true` if the host environment is trusted
- **AND** no TCP listener is ever opened

#### Scenario: Loopback with token enforces bearer authentication
- **WHEN** `quaid serve --http --port 3112 --token-file /home/u/.quaid/http_token` is invoked with `trusted_loopback = false` and a valid token file
- **THEN** the listener is bound to `127.0.0.1:3112`
- **AND** requests with `Authorization: Bearer <token>` matching the file contents succeed
- **AND** requests with a missing or incorrect bearer header are rejected with HTTP 401

#### Scenario: Loopback under trusted_loopback = true is unauthenticated
- **WHEN** `quaid serve --http --port 3112 --trust-loopback` is invoked with no `--token-file`
- **THEN** the listener is bound to `127.0.0.1:3112`
- **AND** unauthenticated MCP connections from localhost succeed
- **AND** the security profile matches stdio (any local process can connect)

### Requirement: Non-loopback bind always requires a token and fails closed without one

If `--bind` resolves to any address other than `127.0.0.1`/`::1` and `--token-file` is not provided, the system SHALL refuse to start and SHALL exit non-zero regardless of `trusted_loopback`. The error SHALL name both the offending bind address and the required `--token-file` flag. This is a startup-time check; the process SHALL NOT listen on the network port and then start rejecting requests — it SHALL never open the listener at all. `trusted_loopback` SHALL NOT apply to non-loopback binds.

#### Scenario: --bind 0.0.0.0 without --token-file refuses to start (even with --trust-loopback)
- **WHEN** `quaid serve --http --bind 0.0.0.0 --trust-loopback` is invoked without `--token-file`
- **THEN** the process exits with status non-zero
- **AND** the error message names both the bind address and the missing `--token-file` argument
- **AND** the message explicitly notes that `trusted_loopback` does not relax non-loopback bind requirements
- **AND** no TCP listener is ever opened on the configured port

#### Scenario: --bind 0.0.0.0 with --token-file succeeds
- **WHEN** `quaid serve --http --bind 0.0.0.0 --port 3112 --token-file /home/u/.quaid/http_token` is invoked with a valid token file
- **THEN** the listener is bound to `0.0.0.0:3112`
- **AND** requests with `Authorization: Bearer <token>` matching the file contents succeed
- **AND** requests with a missing or incorrect bearer header are rejected with HTTP 401

### Requirement: Token file is single-line, ≥32 random bytes, mode 0600

The token file SHALL contain a single line of base64-url-encoded random bytes with at least 32 bytes of entropy. The HTTP transport SHALL refuse to start if the token file is shorter than 32 bytes after decoding, contains more than one non-blank line, or has Unix permission bits that grant read access to group or other (i.e., mode SHALL be `0600` or stricter on Unix; Windows file ACLs are out of scope for v1). All failures SHALL produce actionable, non-leaking error messages — the token contents SHALL NEVER appear in logs.

#### Scenario: Token file with permissive mode is rejected
- **WHEN** the configured `--token-file` has mode `0644` on a Unix system
- **THEN** the process exits with a "token file must be mode 0600" error before opening the listener
- **AND** the token contents do not appear in the error message or any log line

#### Scenario: Token file with insufficient entropy is rejected
- **WHEN** the configured `--token-file` decodes to fewer than 32 bytes
- **THEN** the process exits with a "token file must contain at least 32 bytes of entropy" error
- **AND** the message suggests `openssl rand -base64 32 > <path> && chmod 600 <path>`

### Requirement: SIGHUP reloads the token file without restarting the host process

The HTTP transport SHALL re-read `--token-file` on SIGHUP and use the new contents for subsequent connection authentication. This applies to both `quaid serve --http` and `quaid daemon run --http`. In-flight connections established with the old token SHALL remain valid for the duration of those connections; new connections after SIGHUP SHALL authenticate against the new contents. The SIGHUP re-read SHALL be synchronous and fail closed: if the new file contents fail the same validation as startup, the SIGHUP handler SHALL log an error and continue using the previously-loaded token.

#### Scenario: SIGHUP reloads new valid token
- **WHEN** the host (daemon or serve) is running with a valid token file, the file is updated with a new valid token, and SIGHUP is delivered
- **THEN** subsequent connections authenticated with the new token succeed
- **AND** subsequent connections authenticated with the old token are rejected with HTTP 401

#### Scenario: SIGHUP with invalid new token keeps the old token active
- **WHEN** the host is running with a valid token file and SIGHUP is delivered after the file is rewritten with a 4-byte token
- **THEN** the handler logs `token_reload_failed reason=insufficient_entropy`
- **AND** the previously-loaded token continues to authenticate connections

### Requirement: `rmcp` `transport-sse-server` feature is enabled in Cargo.toml

`Cargo.toml` SHALL list `rmcp` with `features = ["transport-sse-server"]` so the SSE server transport is compiled in. No alternative HTTP layer (Axum, Hyper directly, etc.) SHALL be added; this requirement constrains the dependency surface to a single MCP-aware transport stack.

#### Scenario: Release build includes SSE transport
- **WHEN** `cargo build --release` is run
- **THEN** the resulting binary supports `quaid serve --http` and `quaid daemon run --http`
- **AND** no additional HTTP framework crate is added beyond what `rmcp` pulls

### Requirement: HTTP transport inherits the existing MCP error envelope

Tool errors over HTTP SHALL use the same `rmcp::Error` envelope as stdio — type, code, message — without leaking internal paths, environment variables, or database contents. The HTTP layer SHALL NOT introduce a parallel error formatter; it SHALL share the existing `map_anyhow_error` and `map_db_error` paths used by stdio handlers.

#### Scenario: Error response over HTTP matches stdio shape
- **WHEN** the same invalid arguments are sent to `memory_search` over HTTP and over stdio
- **THEN** both transports return errors with the same code and message structure
- **AND** neither response includes raw paths, stack traces, or DB-internal identifiers
