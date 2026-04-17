use std::path::Path;
use std::sync::Once;

use rusqlite::{params, Connection, OptionalExtension};

use super::inference::{
    coerce_model_for_build, configure_runtime_model, default_model, hydrate_model_config,
    ModelConfig,
};
use super::types::DbError;

static SQLITE_VEC_INIT: Once = Once::new();
const SCHEMA_VERSION: i64 = 4;
const LEGACY_SMALL_MODEL_NAME: &str = "bge-small-en-v1.5";

pub struct OpenDb {
    pub conn: Connection,
    pub effective_model: ModelConfig,
}

impl std::fmt::Debug for OpenDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenDb")
            .field("effective_model", &self.effective_model)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrainConfig {
    pub model_id: String,
    pub model_alias: String,
    pub embedding_dim: usize,
    pub schema_version: i64,
}

impl BrainConfig {
    fn from_model(model: &ModelConfig) -> Self {
        Self {
            model_id: model.model_id.clone(),
            model_alias: model.alias.clone(),
            embedding_dim: model.embedding_dim,
            schema_version: SCHEMA_VERSION,
        }
    }

    fn to_model_config(&self) -> ModelConfig {
        // For standard aliases (small/base/large/m3) resolve via alias to get
        // the correct SHA-256 pins without emitting the "unpinned custom model"
        // warning that resolve_model() would print for an unknown model_id.
        // For custom models, construct directly from persisted values.
        let alias = self.model_alias.as_str();
        if matches!(alias, "small" | "base" | "large" | "m3") {
            let mut model = crate::core::inference::resolve_model(alias);
            model.embedding_dim = self.embedding_dim;
            model
        } else {
            crate::core::inference::ModelConfig {
                alias: self.model_alias.clone(),
                model_id: self.model_id.clone(),
                embedding_dim: self.embedding_dim,
                sha256_hashes: None,
            }
        }
    }
}

fn ensure_sqlite_vec() {
    SQLITE_VEC_INIT.call_once(|| unsafe {
        let init_fn = std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(
                *mut rusqlite::ffi::sqlite3,
                *mut *const std::ffi::c_char,
                *const rusqlite::ffi::sqlite3_api_routines,
            ) -> std::ffi::c_int,
        >(sqlite_vec::sqlite3_vec_init as *const ());
        rusqlite::ffi::sqlite3_auto_extension(Some(init_fn));
    });
}

#[allow(dead_code)]
pub fn open(path: &str) -> Result<Connection, DbError> {
    open_with_model(path, &default_model()).map(|opened| opened.conn)
}

pub fn open_with_model(path: &str, requested_model: &ModelConfig) -> Result<OpenDb, DbError> {
    let requested_model = coerce_model_for_build(requested_model);
    let existed_before = path != ":memory:" && Path::new(path).exists();
    let conn = open_connection(path)?;

    if !existed_before || path == ":memory:" {
        let effective_model = hydrate_model_config(&requested_model)
            .map_err(|message| DbError::Schema { message })?;
        ensure_embedding_model_registry(&conn, &effective_model)?;
        write_brain_config(&conn, &BrainConfig::from_model(&effective_model))?;
        sync_legacy_config(&conn, &effective_model)?;
        configure_runtime_model(effective_model.clone());
        return Ok(OpenDb {
            conn,
            effective_model,
        });
    }

    let effective_model = match read_brain_config(&conn)? {
        Some(stored) => {
            if stored.model_id != requested_model.model_id {
                return Err(DbError::ModelMismatch {
                    message: format_model_mismatch(&stored, &requested_model, path),
                });
            }
            stored.to_model_config()
        }
        None => {
            eprintln!(
                "Warning: brain_config is missing or empty; assuming a legacy BAAI/bge-small-en-v1.5 database. Run `gbrain init` once to persist model metadata."
            );
            upgrade_legacy_small_model_names(&conn)?;
            default_model()
        }
    };

    ensure_embedding_model_registry(&conn, &effective_model)?;
    sync_legacy_config(&conn, &effective_model)?;
    configure_runtime_model(effective_model.clone());

    Ok(OpenDb {
        conn,
        effective_model,
    })
}

pub fn init(path: &str, requested_model: &ModelConfig) -> Result<Connection, DbError> {
    let requested_model = coerce_model_for_build(requested_model);
    let existed_before = path != ":memory:" && Path::new(path).exists();
    let conn = open_connection(path)?;

    if let Some(stored) = read_brain_config(&conn)? {
        let stored_model = stored.to_model_config();
        ensure_embedding_model_registry(&conn, &stored_model)?;
        sync_legacy_config(&conn, &stored_model)?;
        configure_runtime_model(stored_model);
        return Ok(conn);
    }

    let selected_model = if existed_before {
        eprintln!(
            "Warning: brain_config missing or empty on existing database; writing default small-model metadata during `gbrain init`."
        );
        upgrade_legacy_small_model_names(&conn)?;
        default_model()
    } else {
        hydrate_model_config(&requested_model).map_err(|message| DbError::Schema { message })?
    };

    ensure_embedding_model_registry(&conn, &selected_model)?;
    write_brain_config(&conn, &BrainConfig::from_model(&selected_model))?;
    sync_legacy_config(&conn, &selected_model)?;
    configure_runtime_model(selected_model);
    Ok(conn)
}

fn open_connection(path: &str) -> Result<Connection, DbError> {
    let db_path = Path::new(path);
    if let Some(parent) = db_path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            return Err(DbError::PathNotFound {
                path: parent.display().to_string(),
            });
        }
    }

    ensure_sqlite_vec();

    let conn = Connection::open(path)?;
    conn.execute_batch(include_str!("../schema.sql"))?;
    set_version(&conn)?;

    Ok(conn)
}

fn ensure_embedding_model_registry(conn: &Connection, model: &ModelConfig) -> Result<(), DbError> {
    let vec_table = model.vec_table();
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS {vec_table} USING vec0(embedding float[{}]);",
        model.embedding_dim
    ))?;
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO embedding_models (name, dimensions, vec_table, active) \
         VALUES (?1, ?2, ?3, 1) \
          ON CONFLICT(name) DO UPDATE SET \
             dimensions = excluded.dimensions, \
             vec_table = excluded.vec_table, \
             active = excluded.active",
        params![
            model.embedding_model_name(),
            model.embedding_dim as i64,
            vec_table,
        ],
    )?;
    tx.execute(
        "UPDATE embedding_models SET active = 0 WHERE name != ?1 AND active != 0",
        [model.embedding_model_name()],
    )?;
    tx.commit()?;

    Ok(())
}

fn sync_legacy_config(conn: &Connection, model: &ModelConfig) -> Result<(), DbError> {
    conn.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES ('embedding_model', ?1)",
        [model.embedding_model_name()],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES ('embedding_dimensions', ?1)",
        [model.embedding_dim.to_string()],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES ('version', ?1)",
        [SCHEMA_VERSION.to_string()],
    )?;
    Ok(())
}

pub fn write_brain_config(conn: &Connection, config: &BrainConfig) -> Result<(), DbError> {
    // Write all four keys atomically so a mid-flight crash never leaves a
    // partial brain_config that silently falls back to the legacy small-model
    // path on the next open.
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO brain_config (key, value) VALUES ('model_id', ?1) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [&config.model_id],
    )?;
    tx.execute(
        "INSERT INTO brain_config (key, value) VALUES ('model_alias', ?1) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [&config.model_alias],
    )?;
    tx.execute(
        "INSERT INTO brain_config (key, value) VALUES ('embedding_dim', ?1) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [config.embedding_dim.to_string()],
    )?;
    tx.execute(
        "INSERT INTO brain_config (key, value) VALUES ('schema_version', ?1) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [config.schema_version.to_string()],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn read_brain_config(conn: &Connection) -> Result<Option<BrainConfig>, DbError> {
    if !table_exists(conn, "brain_config")? {
        // Legacy DB pre-dating brain_config — treated as small-model default.
        return Ok(None);
    }

    // Fetch all four required keys in one pass.
    let mut rows: std::collections::HashMap<String, String> = conn
        .prepare("SELECT key, value FROM brain_config WHERE key IN ('model_id','model_alias','embedding_dim','schema_version')")?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<Result<_, _>>()?;

    // Table exists but is completely empty → legacy / pre-migration DB.
    if rows.is_empty() {
        return Ok(None);
    }

    // Table is present but missing one or more keys → partial write, treat as error.
    let required = ["model_id", "model_alias", "embedding_dim", "schema_version"];
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|k| !rows.contains_key(*k))
        .collect();
    if !missing.is_empty() {
        return Err(DbError::Schema {
            message: format!(
                "brain_config is incomplete (missing keys: {}). \
                 The database may have been corrupted by an interrupted write. \
                 Re-initialize with: rm <path-to-brain.db> && gbrain init",
                missing.join(", ")
            ),
        });
    }

    let model_id = rows.remove("model_id").unwrap();
    let model_alias = rows.remove("model_alias").unwrap();
    let embedding_dim = rows
        .remove("embedding_dim")
        .unwrap()
        .parse::<usize>()
        .map_err(|_| DbError::Schema {
            message: "brain_config.embedding_dim must be an integer".to_owned(),
        })?;
    let schema_version = rows
        .remove("schema_version")
        .unwrap()
        .parse::<i64>()
        .map_err(|_| DbError::Schema {
            message: "brain_config.schema_version must be an integer".to_owned(),
        })?;

    Ok(Some(BrainConfig {
        model_id,
        model_alias,
        embedding_dim,
        schema_version,
    }))
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool, DbError> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
            [name],
            |row| row.get(0),
        )
        .optional()?;
    Ok(exists.is_some())
}

fn format_model_mismatch(stored: &BrainConfig, requested: &ModelConfig, db_path: &str) -> String {
    let requested_dim = if requested.embedding_dim == 0 {
        "unknown".to_owned()
    } else {
        requested.embedding_dim.to_string()
    };

    format!(
        "Error: Model mismatch\n\n  This brain.db was initialized with: {} ({} dimensions)\n  You requested:                       {} ({} dimensions)\n\n  Embedding dimensions are incompatible. Options:\n    1. Use the original model:   GBRAIN_MODEL={} gbrain <command>\n    2. Re-initialize the DB:     rm {} && gbrain init   (data will be lost)",
        stored.model_id,
        stored.embedding_dim,
        requested.model_id,
        requested_dim,
        if stored.model_alias == "custom" {
            stored.model_id.as_str()
        } else {
            stored.model_alias.as_str()
        },
        db_path,
    )
}

fn upgrade_legacy_small_model_names(conn: &Connection) -> Result<(), DbError> {
    let has_legacy_model: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM embedding_models WHERE name = ?1)",
        [LEGACY_SMALL_MODEL_NAME],
        |row| row.get(0),
    )?;

    if !has_legacy_model {
        return Ok(());
    }

    let default_small = default_model();
    conn.execute(
        "UPDATE page_embeddings SET model = ?1 WHERE model = ?2",
        params![default_small.model_id, LEGACY_SMALL_MODEL_NAME],
    )?;
    conn.execute(
        "UPDATE embedding_models SET name = ?1, dimensions = ?2, vec_table = ?3 \
         WHERE name = ?4",
        params![
            default_small.model_id,
            default_small.embedding_dim as i64,
            default_small.vec_table(),
            LEGACY_SMALL_MODEL_NAME
        ],
    )?;
    conn.execute(
        "UPDATE config SET value = ?1 WHERE key = 'embedding_model' AND value = ?2",
        params![default_small.model_id, LEGACY_SMALL_MODEL_NAME],
    )?;
    Ok(())
}

pub fn compact(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(())
}

pub fn set_version(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::inference::resolve_model;

    #[test]
    fn open_creates_all_expected_tables() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let conn = open(db_path.to_str().unwrap()).unwrap();

        let tables: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master \
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
                 ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();

        let expected = [
            "assertions",
            "brain_config",
            "config",
            "contradictions",
            "embedding_models",
            "import_manifest",
            "ingest_log",
            "knowledge_gaps",
            "links",
            "page_embeddings",
            "page_fts",
            "pages",
            "raw_data",
            "raw_imports",
            "tags",
            "timeline_entries",
        ];

        for name in &expected {
            assert!(
                tables.contains(&(*name).to_string()),
                "missing table: {name}"
            );
        }
    }

    #[test]
    fn open_sets_user_version_to_4() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let conn = open(db_path.to_str().unwrap()).unwrap();

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 4);
    }

    #[test]
    fn open_enables_wal_and_foreign_keys() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let conn = open(db_path.to_str().unwrap()).unwrap();

        let journal: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal.to_lowercase(), "wal");

        let fk: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }

    #[test]
    fn open_rejects_nonexistent_parent_dir() {
        let result = open("/nonexistent/dir/brain.db");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DbError::PathNotFound { .. }));
    }

    #[test]
    fn open_is_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let path_str = db_path.to_str().unwrap();

        let conn1 = open(path_str).unwrap();
        drop(conn1);

        let conn2 = open(path_str).unwrap();
        let version: i64 = conn2
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 4);
    }

    #[test]
    fn compact_checkpoints_wal() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let conn = open(db_path.to_str().unwrap()).unwrap();
        assert!(compact(&conn).is_ok());
    }

    #[test]
    fn open_seeds_default_embedding_model() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let opened = open_with_model(db_path.to_str().unwrap(), &default_model()).unwrap();

        let (name, dims, active): (String, i64, i64) = opened
            .conn
            .query_row(
                "SELECT name, dimensions, active FROM embedding_models WHERE active = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();

        assert_eq!(name, "BAAI/bge-small-en-v1.5");
        assert_eq!(dims, 384);
        assert_eq!(active, 1);
    }

    #[test]
    fn brain_config_roundtrip_preserves_values() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let opened = open_with_model(db_path.to_str().unwrap(), &default_model()).unwrap();

        let config = read_brain_config(&opened.conn).unwrap().unwrap();
        assert_eq!(
            config,
            BrainConfig {
                model_id: "BAAI/bge-small-en-v1.5".to_owned(),
                model_alias: "small".to_owned(),
                embedding_dim: 384,
                schema_version: 4,
            }
        );
    }

    #[test]
    fn empty_brain_config_is_treated_as_legacy_small_model() {
        let conn = open(":memory:").unwrap();
        conn.execute("DELETE FROM brain_config", []).unwrap();

        let config = read_brain_config(&conn).unwrap();
        assert!(config.is_none());
    }

    #[test]
    fn incomplete_brain_config_returns_schema_error() {
        let conn = open(":memory:").unwrap();
        conn.execute("DELETE FROM brain_config", []).unwrap();
        conn.execute(
            "INSERT INTO brain_config (key, value) VALUES ('model_id', 'BAAI/bge-small-en-v1.5')",
            [],
        )
        .unwrap();

        let err = read_brain_config(&conn).unwrap_err();
        assert!(matches!(err, DbError::Schema { .. }));
    }

    #[test]
    fn missing_brain_config_is_treated_as_legacy_small_model() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_brain.db");
        let opened = open_with_model(db_path.to_str().unwrap(), &default_model()).unwrap();
        opened.conn.execute("DELETE FROM brain_config", []).unwrap();
        drop(opened);

        let reopened = open_with_model(db_path.to_str().unwrap(), &resolve_model("large"));
        assert!(reopened.is_ok());
    }

    #[test]
    fn mismatch_detection_returns_model_mismatch_error() {
        let stored = BrainConfig {
            model_id: "BAAI/bge-small-en-v1.5".to_owned(),
            model_alias: "small".to_owned(),
            embedding_dim: 384,
            schema_version: 4,
        };
        let requested = resolve_model("large");
        let message = format_model_mismatch(&stored, &requested, "/tmp/test/brain.db");

        let err = DbError::ModelMismatch { message };
        let DbError::ModelMismatch { message } = &err else {
            unreachable!()
        };
        assert!(message.contains("rm /tmp/test/brain.db && gbrain init"));
    }

    #[cfg(feature = "online-model")]
    #[test]
    fn init_with_small_then_open_with_large_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("brain.db");

        init(db_path.to_str().unwrap(), &resolve_model("small")).unwrap();
        let err = open_with_model(db_path.to_str().unwrap(), &resolve_model("large")).unwrap_err();

        assert!(matches!(err, DbError::ModelMismatch { .. }));
    }

    #[cfg(feature = "online-model")]
    #[test]
    fn init_with_large_then_open_with_large_succeeds() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("brain.db");

        let init_opened =
            open_with_model(db_path.to_str().unwrap(), &resolve_model("large")).unwrap();
        drop(init_opened);

        let reopened = open_with_model(db_path.to_str().unwrap(), &resolve_model("large")).unwrap();
        let stored = read_brain_config(&reopened.conn).unwrap().unwrap();
        assert_eq!(stored.model_alias, "large");
        assert_eq!(stored.embedding_dim, 1024);
    }
}
