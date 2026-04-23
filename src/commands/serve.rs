use anyhow::Result;
use rusqlite::Connection;

pub async fn run(db: Connection) -> Result<()> {
    crate::core::vault_sync::ensure_unix_platform("gbrain serve")
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let db_path = crate::core::vault_sync::database_path(&db)?;
    let _runtime = crate::core::vault_sync::start_serve_runtime(db_path)?;
    crate::mcp::server::run(db).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;

    #[cfg(not(unix))]
    #[test]
    fn serve_requires_unix_platform() {
        let conn = db::open(":memory:").unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();

        let error = runtime.block_on(run(conn)).unwrap_err();

        assert!(error.to_string().contains("UnsupportedPlatformError"));
    }
}
