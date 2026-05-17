use anyhow::{anyhow, Result};
use rusqlite::Connection;

#[cfg(unix)]
use crate::commands::shutdown::ShutdownSignal;
use crate::mcp::HttpConfig;

/// Run `quaid serve` — start the live vault-sync supervisor (when this
/// process becomes runtime owner per the `daemon-and-http-transport`
/// coordination contract) and the MCP transport.
///
/// `http_config = None` → stdio MCP (the default and back-compat path).
/// `http_config = Some(_)` → HTTP/SSE MCP on the configured loopback
/// port. Mutually exclusive: stdio and HTTP are not opened in the same
/// invocation.
pub async fn run(db: Connection, http_config: Option<HttpConfig>) -> Result<()> {
    if let Some(config) = http_config.as_ref() {
        crate::mcp::http::bind_with_token_guard(config)?;
    }

    #[cfg(unix)]
    let mut shutdown_signal = ShutdownSignal::arm();

    let db_path = crate::core::vault_sync::database_path(&db)?;
    // `start_serve_runtime` registers as `serve` and attempts to promote
    // to `serve_host`. If a `daemon` is live it returns a transport-only
    // handle (no watchers, no extraction worker), so the daemon-installed
    // case Just Works without the operator thinking about it.
    let _runtime = crate::core::vault_sync::start_serve_runtime(db_path.clone())?;

    match http_config {
        None => {
            drop(db);
            #[cfg(unix)]
            {
                run_stdio_until_shutdown(db_path, &mut shutdown_signal).await
            }
            #[cfg(not(unix))]
            {
                run_stdio_until_shutdown(db_path).await
            }
        }
        Some(config) => {
            // The HTTP transport opens its own DB connection(s) per
            // incoming SSE session; the `db` we were handed by the
            // outer dispatch is dropped after the runtime startup
            // returns. `db_path` is captured by value in the factory.
            drop(db);
            let factory = move || crate::core::db::open(&db_path).map_err(anyhow::Error::from);
            #[cfg(unix)]
            {
                run_http_until_shutdown(factory, config, &mut shutdown_signal).await
            }
            #[cfg(not(unix))]
            {
                run_http_until_shutdown(factory, config).await
            }
        }
    }
}

#[cfg(unix)]
async fn run_stdio_until_shutdown(
    db_path: String,
    shutdown_signal: &mut ShutdownSignal,
) -> Result<()> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("quaid-mcp-stdio".to_owned())
        .spawn(move || {
            let result = run_stdio_blocking(db_path);
            let _ = sender.send(result);
        })
        .map_err(|error| anyhow!("failed to spawn MCP stdio thread: {error}"))?;

    tokio::select! {
        result = receiver => result
            .map_err(|_| anyhow!("MCP stdio thread terminated without reporting a result"))?,
        () = shutdown_signal.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn run_stdio_until_shutdown(db_path: String) -> Result<()> {
    let db = crate::core::db::open(&db_path)?;
    crate::mcp::server::run_stdio(db).await
}

#[cfg(unix)]
fn run_stdio_blocking(db_path: String) -> Result<()> {
    let db = crate::core::db::open(&db_path)?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| anyhow!("failed to create MCP stdio runtime: {error}"))?
        .block_on(crate::mcp::server::run_stdio(db))
}

#[cfg(unix)]
async fn run_http_until_shutdown<F>(
    factory: F,
    config: HttpConfig,
    shutdown_signal: &mut ShutdownSignal,
) -> Result<()>
where
    F: Fn() -> Result<Connection> + Send + Sync + 'static,
{
    tokio::select! {
        result = crate::mcp::http::run_http(factory, config) => result,
        () = shutdown_signal.recv() => Ok(()),
    }
}

#[cfg(not(unix))]
async fn run_http_until_shutdown<F>(factory: F, config: HttpConfig) -> Result<()>
where
    F: Fn() -> Result<Connection> + Send + Sync + 'static,
{
    crate::mcp::http::run_http(factory, config).await
}
