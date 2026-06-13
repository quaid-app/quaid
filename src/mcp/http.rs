//! Opt-in HTTP/SSE MCP transport.
//!
//! Powered by `rmcp`'s `transport-sse-server` feature. `quaid serve --http`
//! and `quaid daemon run --http` both reach this module; the same
//! [`QuaidServer`] (tool registry) is reused under both transports so
//! every MCP tool works identically over stdio and SSE.
//!
//! # v1 scope and known limitations
//!
//! rmcp 0.1.5's `SseServer::serve_with_config` builds and starts its own
//! axum router internally; the public surface does not expose hooks for
//! injecting tower middleware (e.g., for bearer-auth) between the
//! `Router` and `axum::serve`. v1 therefore implements only the subset
//! of `mcp-http-transport` capability behaviour that rmcp 0.1.5 directly
//! supports:
//!
//! - **Loopback bind (`127.0.0.1` / `::1`) is supported** under
//!   `daemon.http.trusted_loopback = true` (the only effective posture
//!   in this build): unauthenticated localhost access matching stdio's
//!   security profile.
//! - **Non-loopback bind is refused at startup**, satisfying the spec's
//!   "fail closed" rule by the strongest means available — no listener
//!   is ever opened.
//! - **Bearer-auth (`--token-file`) is parsed but not enforced** in v1.
//!   When `--token-file` is supplied alongside a loopback bind, the
//!   token file's existence and mode/entropy are validated at startup
//!   (so misconfigured deployments fail fast) but the actual bearer
//!   check on incoming requests is deferred. Until enforcement lands,
//!   `--token-file` is rejected at startup with an explicit "deferred"
//!   error rather than allowed to give operators a false sense of
//!   security.
//! - **Origin/Host validation** defends the loopback listener against
//!   DNS-rebinding: rmcp 0.1.5 exposes no middleware hook, so the
//!   operator-configured port is owned by a thin guard listener that
//!   parses each connection's first request head, rejects any request
//!   whose `Host` is not loopback or whose `Origin`/`Referer` names a
//!   non-loopback origin, and only then forwards the connection to
//!   rmcp's SSE server on a loopback-only ephemeral port. The first
//!   request on each TCP connection is validated; subsequent requests
//!   reusing an already-validated connection are forwarded as-is
//!   (browsers never share a connection across origins, so a rebinding
//!   attacker cannot piggyback on a legitimate client's connection).
//!
//! Lifting these limitations requires either an rmcp upgrade that
//! exposes a `Router`/`tower::Layer` hook, or replacing rmcp's internal
//! SSE handlers with a Quaid-owned axum app that wraps rmcp's
//! lower-level `RoleServer` directly. Tracked as a follow-up to the
//! `daemon-and-http-transport` change.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;

use rmcp::transport::SseServer;
use rusqlite::Connection;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::mcp::server::QuaidServer;

/// Default port for the HTTP/SSE transport.
pub const DEFAULT_HTTP_PORT: u16 = 3112;
/// Default bind address — loopback only.
pub const DEFAULT_HTTP_BIND: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1));

/// Operator-supplied configuration for the HTTP/SSE transport.
///
/// Constructed by CLI flag parsing in `src/commands/serve.rs` and by
/// the daemon entry point in `src/commands/daemon.rs`; passed to
/// [`run_http`] which delegates the security policy validation to
/// [`bind_with_token_guard`].
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// TCP port to listen on.
    pub port: u16,
    /// Bind address. Loopback (`127.0.0.1`/`::1`) is the default; any
    /// other value is refused in v1 (see module docs).
    pub bind: IpAddr,
    /// Optional path to a bearer token file. Bearer-auth enforcement is
    /// not implemented in v1, so [`bind_with_token_guard`] refuses any
    /// config that sets this (fail-closed; never a silent no-op).
    pub token_file: Option<PathBuf>,
    /// When `true`, loopback binds are unauthenticated (stdio-equivalent
    /// security profile). When `false`, loopback requires a token —
    /// which is itself deferred in v1, so this combination is refused.
    pub trusted_loopback: bool,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_HTTP_PORT,
            bind: DEFAULT_HTTP_BIND,
            token_file: None,
            trusted_loopback: false,
        }
    }
}

impl HttpConfig {
    /// Returns the `SocketAddr` this config would listen on.
    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind, self.port)
    }

    /// Returns `true` when the bind address is the IPv4 or IPv6 loopback.
    pub fn is_loopback(&self) -> bool {
        match self.bind {
            IpAddr::V4(v4) => v4.is_loopback(),
            IpAddr::V6(v6) => v6.is_loopback(),
        }
    }
}

/// Errors raised while validating an [`HttpConfig`] before opening the
/// listener. Every variant produces an actionable operator-facing message
/// and guarantees no listener is opened on the configured port.
#[derive(Debug, Error)]
pub enum HttpConfigError {
    /// `--bind` resolves to a non-loopback address. Refused in v1.
    #[error(
        "HTTP transport refused: --bind {bind} resolves to a non-loopback address. \
         Non-loopback bind requires bearer-auth which is not yet enforced in this build. \
         Re-run with --bind 127.0.0.1 (the default) or run via stdio."
    )]
    NonLoopbackBindUnsupported {
        /// The offending bind address.
        bind: IpAddr,
    },

    /// `--token-file` was supplied but bearer-auth enforcement is not
    /// wired up in v1; refuse rather than give operators a false sense
    /// of security.
    #[error(
        "HTTP transport refused: --token-file was supplied but bearer-auth is not yet \
         enforced in this build. Re-run without --token-file (loopback under \
         --trust-loopback is the only supported HTTP mode in v1)."
    )]
    BearerAuthDeferred,

    /// Loopback bind without `--token-file` AND without `--trust-loopback`
    /// is refused: it would be the unauthenticated path the spec
    /// explicitly fails closed against.
    #[error(
        "HTTP transport refused: loopback without --token-file requires --trust-loopback \
         (sets daemon.http.trusted_loopback=true at the CLI). Loopback is reachable from \
         SSH port-forwards, devcontainers, WSL, and shared-user hosts, so the default \
         is to fail closed. Use --trust-loopback if your host is trusted, or run via stdio."
    )]
    LoopbackUntrustedNoToken,
}

/// Validates [`HttpConfig`] against the v1 security policy. On success,
/// returns the validated `SocketAddr` ready for `SseServer::serve_with_config`.
/// On failure, returns a typed error and guarantees no listener was opened.
///
/// Policy matrix (v1):
///
/// | bind         | token_file | trusted_loopback | outcome                                |
/// |--------------|------------|------------------|----------------------------------------|
/// | loopback     | None       | true             | Ok (unauth, stdio-equivalent)          |
/// | loopback     | None       | false            | Err(LoopbackUntrustedNoToken)          |
/// | loopback     | Some(_)    | any              | Err(BearerAuthDeferred)                |
/// | non-loopback | any        | any              | Err(NonLoopbackBindUnsupported)        |
pub fn bind_with_token_guard(config: &HttpConfig) -> Result<SocketAddr, HttpConfigError> {
    if !config.is_loopback() {
        return Err(HttpConfigError::NonLoopbackBindUnsupported { bind: config.bind });
    }
    if config.token_file.is_some() {
        return Err(HttpConfigError::BearerAuthDeferred);
    }
    if !config.trusted_loopback {
        return Err(HttpConfigError::LoopbackUntrustedNoToken);
    }
    Ok(config.socket_addr())
}

/// Run the MCP transport over HTTP/SSE.
///
/// Validates `config` via [`bind_with_token_guard`], then hands the
/// resulting `SocketAddr` to `rmcp::transport::SseServer`. The function
/// blocks until the SSE server's `CancellationToken` is cancelled —
/// callers (the daemon or `quaid serve` foreground entry) own the
/// cancellation source so SIGTERM-driven shutdown can stop the listener
/// cleanly.
///
/// Each incoming SSE connection is given a freshly-constructed
/// [`QuaidServer`] (cloning the `db_conn_factory` per connection so each
/// transport has its own `Mutex<Connection>` — `rusqlite::Connection` is
/// not `Send` and cannot be shared across the SSE worker tasks safely).
pub async fn run_http(
    db_conn_factory: impl Fn() -> anyhow::Result<Connection> + Send + Sync + 'static,
    config: HttpConfig,
) -> anyhow::Result<()> {
    let bind_addr = bind_with_token_guard(&config)?;

    // The operator-configured port is owned by the Origin/Host guard
    // listener, not by rmcp: bind it first so startup fails fast when the
    // port is taken, exactly as the previous direct rmcp bind did.
    let guard_listener = TcpListener::bind(bind_addr).await?;

    // `SseServer::serve` uses default sse_path="/sse" and
    // post_path="/message" plus a fresh `CancellationToken`. We grab the
    // token from `server.config.ct` after construction so the caller can
    // share-cancel for shutdown. rmcp 0.1.5 binds and serves its axum
    // router internally with no middleware hook, so the SSE server gets a
    // loopback-only ephemeral port and only guard-validated connections
    // are forwarded to it.
    let (server, internal_addr) = serve_sse_on_internal_loopback_port().await?;

    let guard_ct = server.config.ct.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = guard_ct.cancelled() => break,
                accepted = guard_listener.accept() => {
                    match accepted {
                        Ok((stream, _peer)) => {
                            tokio::spawn(guard_connection(stream, internal_addr));
                        }
                        Err(error) => {
                            eprintln!("mcp_http_guard_accept_failed error={error}");
                        }
                    }
                }
            }
        }
    });

    // `with_service` spawns a worker per incoming SSE connection that
    // calls the provider closure to instantiate a fresh service. We
    // construct a new `QuaidServer` per connection because each MCP
    // session needs its own DB connection (`rusqlite::Connection` is
    // not shareable across tasks).
    // `rmcp::SseServer::with_service` requires a `Fn() -> S` provider
    // and gives the closure no way to signal failure, so a per-connection
    // DB-open error has to either panic or fabricate a degraded
    // `QuaidServer`. We log and panic — the panic propagates only
    // inside the SSE connection's tokio task, so other in-flight
    // connections aren't torn down; launchd/systemd's auto-restart
    // catches schema/file errors that affect the whole daemon.
    #[allow(
        clippy::panic,
        reason = "rmcp::SseServer::with_service provider closure has no Result return; logging + panic surfaces fatal per-connection DB errors via the platform supervisor"
    )]
    let inner_ct = server.with_service(move || {
        let conn = db_conn_factory().unwrap_or_else(|err| {
            eprintln!("mcp_http_per_connection_db_open_failed error={err}");
            panic!("per-connection DB open failed: {err}");
        });
        QuaidServer::new(conn)
    });

    // Block until the listener is cancelled. The `CancellationToken` is
    // owned by `server.config.ct` and any clone of it (including
    // `inner_ct` from `with_service`); cancelling either tears down
    // both the listener and the per-connection workers.
    inner_ct.cancelled().await;

    Ok(())
}

/// Start rmcp's SSE server on a loopback-only ephemeral port and return
/// it together with the address the guard listener should forward to.
/// rmcp 0.1.5 does not report the port it actually bound, so a probe
/// listener reserves one first; the tiny bind race is retried.
async fn serve_sse_on_internal_loopback_port() -> std::io::Result<(SseServer, SocketAddr)> {
    let mut last_error = None;
    for _ in 0..16 {
        let probe = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let internal_addr = probe.local_addr()?;
        drop(probe);
        match SseServer::serve(internal_addr).await {
            Ok(server) => return Ok((server, internal_addr)),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                last_error = Some(error);
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            "could not reserve an internal loopback port for the SSE server",
        )
    }))
}

/// Maximum bytes of request head the guard will buffer while looking for
/// the end of the header block.
const MAX_REQUEST_HEAD_BYTES: usize = 16 * 1024;

/// How long the guard waits for a client to finish sending its first
/// request head before dropping the connection.
const REQUEST_HEAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Validate the first request on an accepted connection and either
/// forward the connection (validated head bytes included) to the
/// internal SSE server or answer with `403 Forbidden` and close.
async fn guard_connection(mut client: TcpStream, internal_addr: SocketAddr) {
    let head =
        match tokio::time::timeout(REQUEST_HEAD_TIMEOUT, read_request_head(&mut client)).await {
            Ok(Ok(head)) => head,
            Ok(Err(error)) => {
                eprintln!("mcp_http_guard_read_failed error={error}");
                return;
            }
            Err(_elapsed) => return,
        };

    let head_text = String::from_utf8_lossy(&head);
    if let Err(reason) = validate_loopback_request_head(&head_text) {
        eprintln!("mcp_http_guard_rejected reason={reason}");
        let body = format!("Forbidden: {reason}\n");
        let response = format!(
            "HTTP/1.1 403 Forbidden\r\nconnection: close\r\ncontent-type: text/plain; charset=utf-8\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        let _ = client.write_all(response.as_bytes()).await;
        let _ = client.shutdown().await;
        return;
    }

    let mut upstream = match TcpStream::connect(internal_addr).await {
        Ok(upstream) => upstream,
        Err(error) => {
            eprintln!("mcp_http_guard_upstream_connect_failed error={error}");
            return;
        }
    };
    if upstream.write_all(&head).await.is_err() {
        return;
    }
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
}

/// Read from `client` until the end of the HTTP request head
/// (`\r\n\r\n`, or a bare `\n\n` from lenient clients) is buffered,
/// returning every byte read so the forwarder can replay them verbatim.
async fn read_request_head(client: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 1024];
    loop {
        let read = client.read(&mut chunk).await?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed before request head completed",
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if head_end(&buffer).is_some() {
            return Ok(buffer);
        }
        if buffer.len() > MAX_REQUEST_HEAD_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "request head exceeds the guard's size limit",
            ));
        }
    }
}

fn head_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .or_else(|| {
            buffer
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|index| index + 2)
        })
}

/// Validate an HTTP request head against the loopback-only policy:
/// exactly one `Host` header naming a loopback host, and — when present
/// — `Origin` / `Referer` headers naming loopback origins. Anything
/// else is rejected, which is what defeats DNS-rebinding: a rebound
/// hostname still arrives in `Host` (and browser requests carry the
/// attacker page's `Origin`).
pub fn validate_loopback_request_head(head: &str) -> Result<(), String> {
    let mut host_values = Vec::new();
    let mut origin_values = Vec::new();
    let mut referer_values = Vec::new();
    for line in head.lines().skip(1) {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "host" => host_values.push(value),
            "origin" => origin_values.push(value),
            "referer" => referer_values.push(value),
            _ => {}
        }
    }

    match host_values.as_slice() {
        [] => return Err("missing Host header".to_owned()),
        [host] => {
            if !is_loopback_host(host) {
                return Err(format!("Host `{host}` is not a loopback host"));
            }
        }
        _ => return Err("multiple Host headers".to_owned()),
    }
    for origin in origin_values {
        if !url_has_loopback_host(origin) {
            return Err(format!("Origin `{origin}` is not a loopback origin"));
        }
    }
    for referer in referer_values {
        if !url_has_loopback_host(referer) {
            return Err(format!("Referer `{referer}` is not a loopback origin"));
        }
    }
    Ok(())
}

/// Returns `true` when a `host[:port]` value names the loopback
/// interface: `localhost`, an IPv4 loopback (`127.0.0.0/8`), or the
/// IPv6 loopback (`::1`, bracketed or bare).
pub fn is_loopback_host(value: &str) -> bool {
    let value = value.trim();
    // Bracketed IPv6, optionally with a port: `[::1]` / `[::1]:3112`.
    if let Some(rest) = value.strip_prefix('[') {
        let Some((host, after)) = rest.split_once(']') else {
            return false;
        };
        if !(after.is_empty() || after.starts_with(':')) {
            return false;
        }
        return host.parse::<Ipv6Addr>().is_ok_and(|ip| ip.is_loopback());
    }
    // Bare IPv6 (no port possible without brackets).
    if let Ok(ip) = value.parse::<Ipv6Addr>() {
        return ip.is_loopback();
    }
    // `host[:port]` with at most one colon.
    let host = match value.split_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => host,
        Some(_) => return false,
        None => value,
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
}

/// Returns `true` when an `Origin`/`Referer` value is an `http(s)` URL
/// whose authority is a loopback host. The literal `null` origin and
/// non-HTTP schemes are not loopback.
fn url_has_loopback_host(value: &str) -> bool {
    let value = value.trim();
    let lowered = value.to_ascii_lowercase();
    let rest = if let Some(rest) = lowered.strip_prefix("http://") {
        rest
    } else if let Some(rest) = lowered.strip_prefix("https://") {
        rest
    } else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // Strip URL userinfo if present so `evil@localhost` style tricks
    // cannot smuggle a non-loopback host past the check.
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    is_loopback_host(authority)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use std::net::Ipv4Addr;

    fn loopback_v4() -> IpAddr {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    }

    fn any_v4() -> IpAddr {
        IpAddr::V4(Ipv4Addr::UNSPECIFIED)
    }

    #[test]
    fn loopback_trusted_no_token_is_accepted() {
        let cfg = HttpConfig {
            port: 3112,
            bind: loopback_v4(),
            token_file: None,
            trusted_loopback: true,
        };
        let addr = bind_with_token_guard(&cfg).unwrap();
        assert_eq!(addr.ip(), loopback_v4());
        assert_eq!(addr.port(), 3112);
    }

    #[test]
    fn loopback_untrusted_no_token_is_refused() {
        let cfg = HttpConfig {
            port: 3112,
            bind: loopback_v4(),
            token_file: None,
            trusted_loopback: false,
        };
        match bind_with_token_guard(&cfg) {
            Err(HttpConfigError::LoopbackUntrustedNoToken) => {}
            other => panic!("expected LoopbackUntrustedNoToken, got: {other:?}"),
        }
    }

    #[test]
    fn loopback_with_token_is_refused_in_v1() {
        // v1 known limitation: bearer-auth not yet enforced; fail closed.
        let cfg = HttpConfig {
            port: 3112,
            bind: loopback_v4(),
            token_file: Some(PathBuf::from("/dev/null")),
            trusted_loopback: false,
        };
        match bind_with_token_guard(&cfg) {
            Err(HttpConfigError::BearerAuthDeferred) => {}
            other => panic!("expected BearerAuthDeferred, got: {other:?}"),
        }
    }

    #[test]
    fn loopback_with_token_and_trust_loopback_is_still_refused_in_v1() {
        // Even with --trust-loopback, supplying --token-file under v1
        // returns the bearer-auth-deferred error (token is unenforced).
        let cfg = HttpConfig {
            port: 3112,
            bind: loopback_v4(),
            token_file: Some(PathBuf::from("/dev/null")),
            trusted_loopback: true,
        };
        match bind_with_token_guard(&cfg) {
            Err(HttpConfigError::BearerAuthDeferred) => {}
            other => panic!("expected BearerAuthDeferred, got: {other:?}"),
        }
    }

    #[test]
    fn non_loopback_bind_is_refused_regardless_of_other_flags() {
        for (token, trust) in [
            (None, false),
            (None, true),
            (Some(PathBuf::from("/dev/null")), false),
            (Some(PathBuf::from("/dev/null")), true),
        ] {
            let cfg = HttpConfig {
                port: 3112,
                bind: any_v4(),
                token_file: token.clone(),
                trusted_loopback: trust,
            };
            match bind_with_token_guard(&cfg) {
                Err(HttpConfigError::NonLoopbackBindUnsupported { bind }) => {
                    assert_eq!(bind, any_v4());
                }
                other => panic!(
                    "expected NonLoopbackBindUnsupported for token={token:?} trust={trust}, got: {other:?}"
                ),
            }
        }
    }

    #[test]
    fn ipv6_loopback_is_recognized_as_loopback() {
        let cfg = HttpConfig {
            port: 3112,
            bind: IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
            token_file: None,
            trusted_loopback: true,
        };
        assert!(cfg.is_loopback());
        let addr = bind_with_token_guard(&cfg).unwrap();
        assert_eq!(addr.port(), 3112);
    }
}
