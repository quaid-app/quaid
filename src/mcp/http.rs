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
//!
//! Lifting these limitations requires either an rmcp upgrade that
//! exposes a `Router`/`tower::Layer` hook, or replacing rmcp's internal
//! SSE handlers with a Quaid-owned axum app that wraps rmcp's
//! lower-level `RoleServer` directly. Tracked as a follow-up to the
//! `daemon-and-http-transport` change.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use rmcp::transport::SseServer;
use rusqlite::Connection;
use thiserror::Error;

use crate::core::conversation::slm::LazySlmRunner;
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
    /// Optional path to a bearer token file. Currently parsed and
    /// validated but not enforced — see v1 known limitations.
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

    // `SseServer::serve` uses default sse_path="/sse" and
    // post_path="/message" plus a fresh `CancellationToken`. We grab the
    // token from `server.config.ct` after construction so the caller can
    // share-cancel for shutdown.
    let server = SseServer::serve(bind_addr).await?;

    // `with_service` spawns a worker per incoming SSE connection that
    // calls the provider closure to instantiate a fresh service. We
    // construct a new `QuaidServer` per connection because each MCP
    // session needs its own DB connection (`rusqlite::Connection` is
    // not shareable across tasks).
    //
    // The SLM runner, however, is process-wide: it lazily loads a
    // multi-gigabyte model and caches it for the daemon's lifetime. We
    // build one `Arc<LazySlmRunner>` here and clone the `Arc` into every
    // per-connection `QuaidServer`, so all SSE connections share a single
    // warm model load instead of each reloading their own.
    // `rmcp::SseServer::with_service` requires a `Fn() -> S` provider
    // and gives the closure no way to signal failure, so a per-connection
    // DB-open error has to either panic or fabricate a degraded
    // `QuaidServer`. We log and panic — the panic propagates only
    // inside the SSE connection's tokio task, so other in-flight
    // connections aren't torn down; launchd/systemd's auto-restart
    // catches schema/file errors that affect the whole daemon.
    let shared_slm = Arc::new(LazySlmRunner::new());
    let inner_ct =
        server.with_service(move || build_connection_service(&db_conn_factory, &shared_slm));

    // Block until the listener is cancelled. The `CancellationToken` is
    // owned by `server.config.ct` and any clone of it (including
    // `inner_ct` from `with_service`); cancelling either tears down
    // both the listener and the per-connection workers.
    inner_ct.cancelled().await;

    Ok(())
}

/// Builds the per-SSE-connection [`QuaidServer`]: a fresh DB connection (each
/// transport needs its own non-`Send` `rusqlite::Connection`) paired with a
/// clone of the process-wide `shared_slm` `Arc`, so every connection shares one
/// lazily-loaded model. Extracted so the `with_service` provider closure and
/// tests construct connections identically.
#[allow(
    clippy::panic,
    reason = "rmcp::SseServer::with_service provider closure has no Result return; logging + panic surfaces fatal per-connection DB errors via the platform supervisor"
)]
pub fn build_connection_service(
    db_conn_factory: &(impl Fn() -> anyhow::Result<Connection> + ?Sized),
    shared_slm: &Arc<LazySlmRunner>,
) -> QuaidServer {
    let conn = db_conn_factory().unwrap_or_else(|err| {
        eprintln!("mcp_http_per_connection_db_open_failed error={err}");
        panic!("per-connection DB open failed: {err}");
    });
    QuaidServer::new_with_slm(conn, Arc::clone(shared_slm))
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
