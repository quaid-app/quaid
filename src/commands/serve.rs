use anyhow::Result;
use rusqlite::Connection;

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

    let db_path = crate::core::vault_sync::database_path(&db)?;
    // `start_serve_runtime` registers as `serve` and attempts to promote
    // to `serve_host`. If a `daemon` is live it returns a transport-only
    // handle (no watchers, no extraction worker), so the daemon-installed
    // case Just Works without the operator thinking about it.
    let _runtime = crate::core::vault_sync::start_serve_runtime(db_path.clone())?;

    match http_config {
        None => crate::mcp::server::run_stdio(db).await,
        Some(config) => {
            // The HTTP transport opens its own DB connection(s) per
            // incoming SSE session; the `db` we were handed by the
            // outer dispatch is dropped after the runtime startup
            // returns. `db_path` is captured by value in the factory.
            drop(db);
            let factory = move || crate::core::db::open(&db_path).map_err(anyhow::Error::from);
            crate::mcp::http::run_http(factory, config).await
        }
    }
}
