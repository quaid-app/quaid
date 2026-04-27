use anyhow::Result;
use rusqlite::Connection;

pub async fn run(db: Connection) -> Result<()> {
    // `quaid serve` owns the vault-sync runtime (watchers, leases, startup recovery),
    // so this branch keeps the whole command Unix-gated until a safe non-Unix contract exists.
    crate::core::vault_sync::ensure_unix_platform("quaid serve")
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let db_path = crate::core::vault_sync::database_path(&db)?;
    // start_serve_runtime is cross-platform; watcher threads are #[cfg(unix)]-gated internally.
    let _runtime = crate::core::vault_sync::start_serve_runtime(db_path)?;
    crate::mcp::server::run(db).await
}
