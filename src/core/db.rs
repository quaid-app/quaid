use std::collections::HashSet;
use std::path::Path;
use std::sync::Once;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};

use super::inference::{
    coerce_model_for_build, configure_runtime_model, default_model, hydrate_model_config,
    ModelConfig,
};
use super::types::DbError;

static SQLITE_VEC_INIT: Once = Once::new();
const SCHEMA_VERSION: i64 = 9;
const PAGES_AU_QUARANTINE_GUARD: &str = "WHERE old.quarantined_at IS NULL";
const PAGES_AU_TRIGGER_SQL: &str =
    "CREATE TRIGGER IF NOT EXISTS pages_au AFTER UPDATE ON pages BEGIN
    INSERT INTO page_fts(page_fts, rowid, title, slug, compiled_truth, timeline)
    SELECT 'delete', old.id, old.title, old.slug, old.compiled_truth, old.timeline
    WHERE old.quarantined_at IS NULL;
    INSERT INTO page_fts(rowid, title, slug, compiled_truth, timeline)
    SELECT new.id, new.title, new.slug, new.compiled_truth, new.timeline
    WHERE new.quarantined_at IS NULL;
END;";

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
pub struct QuaidConfig {
    pub model_id: String,
    pub model_alias: String,
    pub embedding_dim: usize,
    pub schema_version: i64,
}

impl QuaidConfig {
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

pub fn default_db_path() -> std::path::PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".quaid").join("memory.db"))
        .unwrap_or_else(|| std::path::PathBuf::from("memory.db"))
}

pub fn default_db_path_string() -> String {
    default_db_path().display().to_string()
}

pub fn open_with_model(path: &str, requested_model: &ModelConfig) -> Result<OpenDb, DbError> {
    let requested_model = coerce_model_for_build(requested_model);
    let existed_before = path != ":memory:" && Path::new(path).exists();
    preflight_existing_schema(path)?;
    let conn = open_connection(path)?;

    if !existed_before || path == ":memory:" {
        let effective_model = hydrate_model_config(&requested_model)
            .map_err(|message| DbError::Schema { message })?;
        persist_model_metadata(&conn, &effective_model)?;
        configure_runtime_model(effective_model.clone());
        return Ok(OpenDb {
            conn,
            effective_model,
        });
    }

    let effective_model = match read_quaid_config(&conn)? {
        Some(stored) => {
            // Check schema version — refuse to open older schema versions
            if stored.schema_version < SCHEMA_VERSION {
                return Err(DbError::Schema {
                    message: format_schema_reinit_message(stored.schema_version, path),
                });
            }
            if stored.model_id != requested_model.model_id {
                return Err(DbError::ModelMismatch {
                    message: format_model_mismatch(&stored, &requested_model, path),
                });
            }
            stored.to_model_config()
        }
        None => {
            recover_crash_partial_fresh_db(&conn, &requested_model, path)?.ok_or_else(|| {
                DbError::Schema {
                    message: format_schema_reinit_message(0, path),
                }
            })?
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
    preflight_existing_schema(path)?;
    let conn = open_connection(path)?;

    if let Some(stored) = read_quaid_config(&conn)? {
        let stored_model = stored.to_model_config();
        ensure_embedding_model_registry(&conn, &stored_model)?;
        sync_legacy_config(&conn, &stored_model)?;
        configure_runtime_model(stored_model);
        return Ok(conn);
    }

    if existed_before {
        if let Some(recovered_model) =
            recover_crash_partial_fresh_db(&conn, &requested_model, path)?
        {
            configure_runtime_model(recovered_model);
            return Ok(conn);
        }
    }

    if existed_before {
        return Err(DbError::Schema {
            message: format_schema_reinit_message(0, path),
        });
    }

    let selected_model =
        hydrate_model_config(&requested_model).map_err(|message| DbError::Schema { message })?;

    persist_model_metadata(&conn, &selected_model)?;
    configure_runtime_model(selected_model);
    Ok(conn)
}

fn preflight_existing_schema(path: &str) -> Result<(), DbError> {
    if path == ":memory:" || !Path::new(path).exists() {
        return Ok(());
    }

    let conn = Connection::open(path)?;
    let Some(schema_version) = read_existing_schema_version(&conn)? else {
        return Ok(());
    };

    if schema_version < SCHEMA_VERSION {
        return Err(DbError::Schema {
            message: format_schema_reinit_message(schema_version, path),
        });
    }

    Ok(())
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
    // Set busy timeout *before* schema DDL so concurrent opens don't race on the
    // write lock required by the initial PRAGMA + CREATE TABLE IF NOT EXISTS batch.
    conn.busy_timeout(Duration::from_secs(5))?;
    ensure_namespace_schema(&conn)?;
    conn.execute_batch(include_str!("../schema.sql"))?;
    ensure_pages_update_trigger_handles_quarantine(&conn)?;
    ensure_namespace_schema(&conn)?;
    ensure_collection_owner_columns(&conn)?;
    ensure_serve_session_columns(&conn)?;
    ensure_collection_name_guards(&conn)?;
    ensure_raw_import_hash_schema(&conn)?;
    set_version(&conn)?;
    ensure_default_collection(&conn)?;

    Ok(conn)
}

fn ensure_namespace_schema(conn: &Connection) -> Result<(), DbError> {
    if !table_exists(conn, "pages")? {
        return Ok(());
    }

    let mut stmt = conn.prepare("PRAGMA table_info(pages)")?;
    let existing_columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<HashSet<_>, _>>()?;

    if !existing_columns.contains("namespace") {
        conn.execute_batch("ALTER TABLE pages ADD COLUMN namespace TEXT NOT NULL DEFAULT '';")?;
    }

    if pages_needs_namespace_unique_rebuild(conn)? {
        rebuild_pages_with_namespace_unique(conn)?;
    }

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS namespaces (
             id         TEXT PRIMARY KEY,
             ttl_hours  REAL DEFAULT NULL,
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
         ) STRICT;
         CREATE INDEX IF NOT EXISTS idx_pages_namespace ON pages(namespace);
         DROP INDEX IF EXISTS idx_pages_collection_namespace_slug;",
    )?;

    Ok(())
}

fn pages_needs_namespace_unique_rebuild(conn: &Connection) -> Result<bool, DbError> {
    let mut stmt = conn.prepare(
        "SELECT name, origin
         FROM pragma_index_list('pages')
         WHERE \"unique\" = 1",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut has_namespace_table_constraint = false;
    let mut has_legacy_slug_constraint = false;
    for row in rows {
        let (name, origin) = row?;
        let columns = index_columns(conn, &name)?;
        if columns == ["collection_id", "namespace", "slug"] && origin == "u" {
            has_namespace_table_constraint = true;
        }
        if columns == ["collection_id", "slug"] {
            has_legacy_slug_constraint = true;
        }
    }

    Ok(has_legacy_slug_constraint || !has_namespace_table_constraint)
}

fn index_columns(conn: &Connection, index_name: &str) -> Result<Vec<String>, DbError> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")?;
    let columns = stmt
        .query_map([index_name], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(columns)
}

fn rebuild_pages_with_namespace_unique(conn: &Connection) -> Result<(), DbError> {
    let foreign_keys_enabled: i64 = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;

    conn.execute_batch(
        "PRAGMA foreign_keys = OFF;
         BEGIN;
         DROP TRIGGER IF EXISTS pages_ai;
         DROP TRIGGER IF EXISTS pages_ad;
         DROP TRIGGER IF EXISTS pages_au;
         CREATE TABLE pages_new (
             id              INTEGER PRIMARY KEY AUTOINCREMENT,
             collection_id   INTEGER NOT NULL DEFAULT 1 REFERENCES collections(id) ON DELETE CASCADE,
             namespace       TEXT    NOT NULL DEFAULT '',
             slug            TEXT    NOT NULL,
             uuid            TEXT    DEFAULT NULL,
             type            TEXT    NOT NULL,
             title           TEXT    NOT NULL,
             summary         TEXT    NOT NULL DEFAULT '',
             compiled_truth  TEXT    NOT NULL DEFAULT '',
             timeline        TEXT    NOT NULL DEFAULT '',
             frontmatter     TEXT    NOT NULL DEFAULT '{}',
             wing            TEXT    NOT NULL DEFAULT '',
             room            TEXT    NOT NULL DEFAULT '',
             superseded_by   INTEGER DEFAULT NULL REFERENCES pages(id),
             version         INTEGER NOT NULL DEFAULT 1,
             quarantined_at  TEXT    DEFAULT NULL,
             created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
             updated_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
             truth_updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
             timeline_updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
             UNIQUE(collection_id, namespace, slug)
         );
         INSERT INTO pages_new (
             id, collection_id, namespace, slug, uuid, type, title, summary,
             compiled_truth, timeline, frontmatter, wing, room, superseded_by, version,
             quarantined_at, created_at, updated_at, truth_updated_at,
             timeline_updated_at
         )
         SELECT
             id, collection_id, namespace, slug, uuid, type, title, summary,
             compiled_truth, timeline, frontmatter, wing, room, superseded_by, version,
             quarantined_at, created_at, updated_at, truth_updated_at,
             timeline_updated_at
         FROM pages;
         DROP TABLE pages;
         ALTER TABLE pages_new RENAME TO pages;
         COMMIT;",
    )?;

    if foreign_keys_enabled != 0 {
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    }
    conn.execute_batch("PRAGMA foreign_key_check;")?;

    Ok(())
}

fn ensure_pages_update_trigger_handles_quarantine(conn: &Connection) -> Result<(), DbError> {
    let trigger_sql: Option<String> = conn
        .query_row(
            "SELECT sql
             FROM sqlite_master
             WHERE type = 'trigger' AND name = 'pages_au'",
            [],
            |row| row.get(0),
        )
        .optional()?;

    if trigger_sql
        .as_deref()
        .is_some_and(|sql| sql.contains(PAGES_AU_QUARANTINE_GUARD))
    {
        return Ok(());
    }

    conn.execute_batch(&format!(
        "DROP TRIGGER IF EXISTS pages_au;
         {PAGES_AU_TRIGGER_SQL}"
    ))?;

    Ok(())
}

fn ensure_collection_owner_columns(conn: &Connection) -> Result<(), DbError> {
    let mut stmt = conn.prepare("PRAGMA table_info(collections)")?;
    let existing_columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<HashSet<_>, _>>()?;

    for (column_name, column_sql) in [
        (
            "active_lease_session_id",
            "ALTER TABLE collections ADD COLUMN active_lease_session_id TEXT DEFAULT NULL",
        ),
        (
            "restore_command_id",
            "ALTER TABLE collections ADD COLUMN restore_command_id TEXT DEFAULT NULL",
        ),
        (
            "restore_lease_session_id",
            "ALTER TABLE collections ADD COLUMN restore_lease_session_id TEXT DEFAULT NULL",
        ),
        (
            "reload_generation",
            "ALTER TABLE collections ADD COLUMN reload_generation INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "watcher_released_session_id",
            "ALTER TABLE collections ADD COLUMN watcher_released_session_id TEXT DEFAULT NULL",
        ),
        (
            "watcher_released_generation",
            "ALTER TABLE collections ADD COLUMN watcher_released_generation INTEGER DEFAULT NULL",
        ),
        (
            "watcher_released_at",
            "ALTER TABLE collections ADD COLUMN watcher_released_at TEXT DEFAULT NULL",
        ),
        (
            "pending_command_heartbeat_at",
            "ALTER TABLE collections ADD COLUMN pending_command_heartbeat_at TEXT DEFAULT NULL",
        ),
        (
            "pending_root_path",
            "ALTER TABLE collections ADD COLUMN pending_root_path TEXT DEFAULT NULL",
        ),
        (
            "pending_restore_manifest",
            "ALTER TABLE collections ADD COLUMN pending_restore_manifest TEXT DEFAULT NULL",
        ),
        (
            "restore_command_pid",
            "ALTER TABLE collections ADD COLUMN restore_command_pid INTEGER DEFAULT NULL",
        ),
        (
            "restore_command_host",
            "ALTER TABLE collections ADD COLUMN restore_command_host TEXT DEFAULT NULL",
        ),
        (
            "integrity_failed_at",
            "ALTER TABLE collections ADD COLUMN integrity_failed_at TEXT DEFAULT NULL",
        ),
        (
            "pending_manifest_incomplete_at",
            "ALTER TABLE collections ADD COLUMN pending_manifest_incomplete_at TEXT DEFAULT NULL",
        ),
        (
            "reconcile_halted_at",
            "ALTER TABLE collections ADD COLUMN reconcile_halted_at TEXT DEFAULT NULL",
        ),
        (
            "reconcile_halt_reason",
            "ALTER TABLE collections ADD COLUMN reconcile_halt_reason TEXT DEFAULT NULL",
        ),
    ] {
        if !existing_columns.contains(column_name) {
            conn.execute_batch(column_sql)?;
        }
    }

    Ok(())
}

fn ensure_serve_session_columns(conn: &Connection) -> Result<(), DbError> {
    let mut stmt = conn.prepare("PRAGMA table_info(serve_sessions)")?;
    let existing_columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<HashSet<_>, _>>()?;

    for (column_name, column_sql) in [(
        "session_type",
        "ALTER TABLE serve_sessions ADD COLUMN session_type TEXT NOT NULL DEFAULT 'serve'",
    )] {
        if !existing_columns.contains(column_name) {
            conn.execute_batch(column_sql)?;
        }
    }

    Ok(())
}

fn ensure_collection_name_guards(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS collections_name_reject_double_colon_insert
         BEFORE INSERT ON collections
         WHEN instr(NEW.name, '::') > 0
         BEGIN
             SELECT RAISE(ABORT, 'collections.name cannot contain ::');
         END;

         CREATE TRIGGER IF NOT EXISTS collections_name_reject_double_colon_update
         BEFORE UPDATE OF name ON collections
         WHEN instr(NEW.name, '::') > 0
         BEGIN
             SELECT RAISE(ABORT, 'collections.name cannot contain ::');
         END;",
    )?;
    Ok(())
}

fn ensure_raw_import_hash_schema(conn: &Connection) -> Result<(), DbError> {
    if !table_exists(conn, "raw_imports")? {
        return Ok(());
    }

    let mut stmt = conn.prepare("PRAGMA table_info(raw_imports)")?;
    let existing_columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<HashSet<_>, _>>()?;

    let mut added_content_hash = false;
    if !existing_columns.contains("content_hash") {
        conn.execute_batch(
            "ALTER TABLE raw_imports ADD COLUMN content_hash TEXT NOT NULL DEFAULT '';",
        )?;
        added_content_hash = true;
    }

    if !index_exists(conn, "idx_raw_imports_content_hash")? {
        conn.execute_batch(
            "CREATE INDEX idx_raw_imports_content_hash
             ON raw_imports(content_hash)
             WHERE content_hash != '';",
        )?;
    }

    if !added_content_hash {
        return Ok(());
    }

    const BACKFILL_BATCH_SIZE: i64 = 128;
    loop {
        let rows_to_backfill: Vec<(i64, Vec<u8>)> = conn
            .prepare(
                "SELECT id, raw_bytes
                 FROM raw_imports
                 WHERE content_hash = ''
                 ORDER BY id
                 LIMIT ?1",
            )?
            .query_map([BACKFILL_BATCH_SIZE], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;

        if rows_to_backfill.is_empty() {
            break;
        }

        let tx = conn.unchecked_transaction()?;
        for (id, raw_bytes) in rows_to_backfill {
            tx.execute(
                "UPDATE raw_imports
                 SET content_hash = ?1
                 WHERE id = ?2",
                params![crate::core::raw_imports::content_hash_hex(&raw_bytes), id],
            )?;
        }
        tx.commit()?;
    }

    Ok(())
}

/// Ensure a collection with id=1 exists in the database.
///
/// All legacy INSERT INTO pages statements that omit collection_id rely on
/// `DEFAULT 1` routing them to this collection.  Called at every
/// `open_connection()` so test-only in-memory databases are also covered.
fn ensure_default_collection(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        "INSERT OR IGNORE INTO collections \
             (id, name, root_path, state, writable, is_write_target) \
         VALUES (1, 'default', '', 'detached', 1, 1);",
    )?;
    Ok(())
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

fn persist_model_metadata(conn: &Connection, model: &ModelConfig) -> Result<(), DbError> {
    ensure_embedding_model_registry(conn, model)?;
    write_quaid_config(conn, &QuaidConfig::from_model(model))?;
    sync_legacy_config(conn, model)?;
    Ok(())
}

fn recover_crash_partial_fresh_db(
    conn: &Connection,
    requested_model: &ModelConfig,
    db_path: &str,
) -> Result<Option<ModelConfig>, DbError> {
    if !is_bootstrap_fresh_db(conn)? {
        return Ok(None);
    }

    let effective_model = match read_bootstrap_registry_model(conn)? {
        Some(stored) => {
            if stored.model_id != requested_model.model_id {
                return Err(DbError::ModelMismatch {
                    message: format_model_mismatch(&stored, requested_model, db_path),
                });
            }
            stored.to_model_config()
        }
        None => {
            hydrate_model_config(requested_model).map_err(|message| DbError::Schema { message })?
        }
    };

    persist_model_metadata(conn, &effective_model)?;
    Ok(Some(effective_model))
}

fn is_bootstrap_fresh_db(conn: &Connection) -> Result<bool, DbError> {
    let default_collection_count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM collections
         WHERE id = 1
           AND name = 'default'
           AND root_path = ''
           AND state = 'detached'
           AND writable = 1
           AND is_write_target = 1",
        [],
        |row| row.get(0),
    )?;
    if default_collection_count != 1 {
        return Ok(false);
    }

    let collection_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM collections", [], |row| row.get(0))?;
    if collection_count != 1 {
        return Ok(false);
    }

    let embedding_model_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM embedding_models", [], |row| {
            row.get(0)
        })?;
    if embedding_model_count > 1 {
        return Ok(false);
    }

    let inactive_embedding_model_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM embedding_models WHERE active != 1",
        [],
        |row| row.get(0),
    )?;
    if inactive_embedding_model_count != 0 {
        return Ok(false);
    }

    for table in [
        "assertions",
        "collection_owners",
        "contradictions",
        "embedding_jobs",
        "file_state",
        "import_manifest",
        "knowledge_gaps",
        "links",
        "page_embeddings",
        "pages",
        "quarantine_exports",
        "raw_data",
        "raw_imports",
        "serve_sessions",
        "tags",
        "timeline_entries",
    ] {
        if table_has_rows(conn, table)? {
            return Ok(false);
        }
    }

    Ok(true)
}

fn table_has_rows(conn: &Connection, table: &str) -> Result<bool, DbError> {
    let exists: Option<i64> = conn
        .query_row(&format!("SELECT 1 FROM {table} LIMIT 1"), [], |row| {
            row.get(0)
        })
        .optional()?;
    Ok(exists.is_some())
}

fn read_bootstrap_registry_model(conn: &Connection) -> Result<Option<QuaidConfig>, DbError> {
    conn.query_row(
        "SELECT name, dimensions
         FROM embedding_models
         WHERE active = 1
         LIMIT 1",
        [],
        |row| {
            let model_id: String = row.get(0)?;
            let embedding_dim: i64 = row.get(1)?;
            Ok(QuaidConfig {
                model_alias: model_alias_for_model_id(&model_id).to_owned(),
                model_id,
                embedding_dim: embedding_dim as usize,
                schema_version: SCHEMA_VERSION,
            })
        },
    )
    .optional()
    .map_err(DbError::from)
}

fn model_alias_for_model_id(model_id: &str) -> &'static str {
    match model_id.trim().to_ascii_lowercase().as_str() {
        "baai/bge-small-en-v1.5" => "small",
        "baai/bge-base-en-v1.5" => "base",
        "baai/bge-large-en-v1.5" => "large",
        "baai/bge-m3" => "m3",
        _ => "custom",
    }
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

pub fn write_quaid_config(conn: &Connection, config: &QuaidConfig) -> Result<(), DbError> {
    // Write all four keys atomically so a mid-flight crash never leaves a
    // partial quaid_config that silently falls back to the legacy small-model
    // path on the next open.
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO quaid_config (key, value) VALUES ('model_id', ?1) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [&config.model_id],
    )?;
    tx.execute(
        "INSERT INTO quaid_config (key, value) VALUES ('model_alias', ?1) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [&config.model_alias],
    )?;
    tx.execute(
        "INSERT INTO quaid_config (key, value) VALUES ('embedding_dim', ?1) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [config.embedding_dim.to_string()],
    )?;
    tx.execute(
        "INSERT INTO quaid_config (key, value) VALUES ('schema_version', ?1) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [config.schema_version.to_string()],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn read_quaid_config(conn: &Connection) -> Result<Option<QuaidConfig>, DbError> {
    if !table_exists(conn, "quaid_config")? {
        // Legacy DB pre-dating quaid_config — treated as small-model default.
        return Ok(None);
    }

    // Fetch all four required keys in one pass.
    let mut rows: std::collections::HashMap<String, String> = conn
        .prepare("SELECT key, value FROM quaid_config WHERE key IN ('model_id','model_alias','embedding_dim','schema_version')")?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<Result<_, _>>()?;

    // Table exists but is completely empty → bootstrap-partial or legacy state.
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
                "quaid_config is incomplete (missing keys: {}). \
                 The database may have been corrupted by an interrupted write. \
                 Re-initialize with: rm <path-to-memory.db> && quaid init",
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
            message: "quaid_config.embedding_dim must be an integer".to_owned(),
        })?;
    let schema_version = rows
        .remove("schema_version")
        .unwrap()
        .parse::<i64>()
        .map_err(|_| DbError::Schema {
            message: "quaid_config.schema_version must be an integer".to_owned(),
        })?;

    Ok(Some(QuaidConfig {
        model_id,
        model_alias,
        embedding_dim,
        schema_version,
    }))
}

pub fn read_config_value(conn: &Connection, key: &str) -> Result<Option<String>, DbError> {
    conn.query_row("SELECT value FROM config WHERE key = ?1", [key], |row| {
        row.get(0)
    })
    .optional()
    .map_err(DbError::from)
}

pub fn read_config_value_or(
    conn: &Connection,
    key: &str,
    default: &str,
) -> Result<String, DbError> {
    Ok(read_config_value(conn, key)?.unwrap_or_else(|| default.to_owned()))
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

fn index_exists(conn: &Connection, name: &str) -> Result<bool, DbError> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1 LIMIT 1",
            [name],
            |row| row.get(0),
        )
        .optional()?;
    Ok(exists.is_some())
}

fn read_existing_schema_version(conn: &Connection) -> Result<Option<i64>, DbError> {
    for (table, key) in [("quaid_config", "schema_version"), ("config", "version")] {
        if !table_exists(conn, table)? {
            continue;
        }

        let value: Option<String> = conn
            .query_row(
                &format!("SELECT value FROM {table} WHERE key = ?1"),
                [key],
                |row| row.get(0),
            )
            .optional()?;

        let Some(value) = value else {
            continue;
        };

        let schema_version = value.parse::<i64>().map_err(|_| DbError::Schema {
            message: format!("{table}.{key} must be an integer"),
        })?;
        return Ok(Some(schema_version));
    }

    Ok(None)
}

fn format_schema_reinit_message(schema_version: i64, path: &str) -> String {
    let default_path = default_db_path_string();
    format!(
        "Error: database schema version mismatch.\n  Found version {}, expected {}.\n  Existing databases created before the Quaid rename are not supported.\n  To migrate: export your data with the pre-rename binary, then re-ingest the exported markdown with the current workflow.\n    quaid init {}\n    quaid collection add migrated <export-directory>\n    # or ingest files individually with `quaid ingest`\n  Original database: {}",
        schema_version, SCHEMA_VERSION, default_path, path
    )
}

fn format_model_mismatch(stored: &QuaidConfig, requested: &ModelConfig, db_path: &str) -> String {
    let requested_dim = if requested.embedding_dim == 0 {
        "unknown".to_owned()
    } else {
        requested.embedding_dim.to_string()
    };

    format!(
        "Error: Model mismatch\n\n  This memory.db was initialized with: {} ({} dimensions)\n  You requested:                       {} ({} dimensions)\n\n  Embedding dimensions are incompatible. Options:\n    1. Use the original model:   QUAID_MODEL={} quaid <command>\n    2. Re-initialize the DB:     rm {} && quaid init   (data will be lost)",
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

    fn seed_existing_db(path: &Path, schema_version: i64) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE quaid_config (
                 key   TEXT PRIMARY KEY NOT NULL,
                 value TEXT NOT NULL
             ) STRICT;
             CREATE TABLE config (
                 key   TEXT PRIMARY KEY NOT NULL,
                 value TEXT NOT NULL
             ) STRICT;",
        )
        .unwrap();
        let model = default_model();
        write_quaid_config(
            &conn,
            &QuaidConfig {
                model_id: model.model_id.clone(),
                model_alias: model.alias.clone(),
                embedding_dim: model.embedding_dim,
                schema_version,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO config (key, value) VALUES ('version', ?1)",
            [schema_version.to_string()],
        )
        .unwrap();
    }

    #[test]
    fn open_creates_all_expected_tables() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
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
            "quaid_config",
            "collections",
            "collection_owners",
            "config",
            "contradictions",
            "correction_sessions",
            "embedding_jobs",
            "embedding_models",
            "extraction_queue",
            "file_state",
            "import_manifest",
            "knowledge_gaps",
            "links",
            "namespaces",
            "page_embeddings",
            "page_fts",
            "pages",
            "raw_data",
            "raw_imports",
            "serve_sessions",
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
    fn open_sets_user_version_to_9() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
        let conn = open(db_path.to_str().unwrap()).unwrap();

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 9);
    }

    #[test]
    fn fresh_v9_schema_includes_conversation_memory_artifacts_and_defaults() {
        let conn = open(":memory:").unwrap();

        let page_columns: Vec<String> = conn
            .prepare("PRAGMA table_info(pages)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(page_columns.contains(&"superseded_by".to_string()));

        let page_foreign_keys: Vec<(String, String)> = conn
            .prepare("PRAGMA foreign_key_list(pages)")
            .unwrap()
            .query_map([], |row| Ok((row.get(2)?, row.get(3)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(page_foreign_keys.contains(&("pages".to_string(), "superseded_by".to_string())));

        let supersede_index_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'idx_pages_supersede_head'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(supersede_index_sql.contains("WHERE superseded_by IS NULL"));

        let session_index_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'idx_pages_session'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(session_index_sql.contains("json_valid(frontmatter)"));
        assert!(session_index_sql.contains("$.session_id"));

        let queue_table_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'extraction_queue'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(queue_table_sql.contains("trigger_kind IN ('debounce', 'session_close', 'manual')"));
        assert!(queue_table_sql.contains("status IN ('pending', 'running', 'done', 'failed')"));

        let pending_index_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'idx_extraction_queue_pending'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(pending_index_sql.contains("WHERE status = 'pending'"));

        let correction_table_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'correction_sessions'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(correction_table_sql.contains(
            "status IN ('open', 'committed', 'abandoned', 'expired')"
        ));
        assert!(correction_table_sql.contains("json_valid(exchange_log)"));

        let correction_index_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'idx_correction_open'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(correction_index_sql.contains("WHERE status = 'open'"));

        let config_rows: Vec<(String, String)> = conn
            .prepare(
                "SELECT key, value FROM config
                 WHERE key IN (
                     'version',
                     'memory.location',
                     'corrections.history_on_disk',
                     'extraction.max_retries',
                     'extraction.enabled',
                     'extraction.model_alias',
                     'extraction.window_turns',
                     'extraction.debounce_ms',
                     'extraction.idle_close_ms',
                     'extraction.retention_days',
                     'fact_resolution.dedup_cosine_min',
                     'fact_resolution.supersede_cosine_min'
                 )
                 ORDER BY key",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            config_rows,
            vec![
                (
                    "corrections.history_on_disk".to_string(),
                    "false".to_string()
                ),
                ("extraction.debounce_ms".to_string(), "5000".to_string()),
                ("extraction.enabled".to_string(), "false".to_string()),
                ("extraction.idle_close_ms".to_string(), "60000".to_string()),
                ("extraction.max_retries".to_string(), "3".to_string()),
                (
                    "extraction.model_alias".to_string(),
                    "phi-3.5-mini".to_string()
                ),
                ("extraction.retention_days".to_string(), "30".to_string()),
                ("extraction.window_turns".to_string(), "5".to_string()),
                (
                    "fact_resolution.dedup_cosine_min".to_string(),
                    "0.92".to_string()
                ),
                (
                    "fact_resolution.supersede_cosine_min".to_string(),
                    "0.4".to_string()
                ),
                ("memory.location".to_string(), "vault-subdir".to_string()),
                ("version".to_string(), "9".to_string()),
            ]
        );
    }

    #[test]
    fn fresh_v9_schema_enforces_superseded_by_foreign_key() {
        let conn = open(":memory:").unwrap();

        let err = conn
            .execute(
                "INSERT INTO pages
                     (slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, superseded_by, version)
                 VALUES ('notes/invalid-supersede', ?1, 'concept', 'Invalid', '', '', '', '{}', 'notes', '', 9999, 1)",
                [uuid::Uuid::now_v7().to_string()],
            )
            .expect_err("invalid superseded_by should fail");

        assert!(matches!(err, rusqlite::Error::SqliteFailure(_, _)));
    }

    #[test]
    fn fresh_v9_schema_rejects_invalid_extraction_queue_trigger_kind() {
        let conn = open(":memory:").unwrap();

        let err = conn
            .execute(
                "INSERT INTO extraction_queue
                     (session_id, conversation_path, trigger_kind, enqueued_at, scheduled_for, status)
                 VALUES ('s1', 'conversations/2026-05-04/s1.md', 'arbitrary', '2026-05-04T00:00:00Z', '2026-05-04T00:00:05Z', 'pending')",
                [],
            )
            .expect_err("invalid trigger_kind should fail");

        assert!(matches!(err, rusqlite::Error::SqliteFailure(_, _)));
    }

    #[test]
    fn fresh_v9_schema_rejects_invalid_extraction_queue_status() {
        let conn = open(":memory:").unwrap();

        let err = conn
            .execute(
                "INSERT INTO extraction_queue
                     (session_id, conversation_path, trigger_kind, enqueued_at, scheduled_for, status)
                 VALUES ('s1', 'conversations/2026-05-04/s1.md', 'debounce', '2026-05-04T00:00:00Z', '2026-05-04T00:00:05Z', 'queued')",
                [],
            )
            .expect_err("invalid status should fail");

        assert!(matches!(err, rusqlite::Error::SqliteFailure(_, _)));
    }

    #[test]
    fn fresh_v9_schema_rejects_invalid_correction_session_status() {
        let conn = open(":memory:").unwrap();

        let err = conn
            .execute(
                "INSERT INTO correction_sessions
                     (correction_id, fact_slug, exchange_log, turns_used, status, created_at, expires_at)
                 VALUES (?1, 'facts/example', '[]', 0, 'paused', '2026-05-05T00:00:00Z', '2026-05-05T01:00:00Z')",
                [uuid::Uuid::now_v7().to_string()],
            )
            .expect_err("invalid correction session status should fail");

        assert!(matches!(err, rusqlite::Error::SqliteFailure(_, _)));
    }

    #[test]
    fn open_enables_wal_and_foreign_keys() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
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
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("missing-parent").join("memory.db");
        let result = open(missing.to_str().unwrap());
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DbError::PathNotFound { .. }));
    }

    #[test]
    fn open_is_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
        let path_str = db_path.to_str().unwrap();

        let conn1 = open(path_str).unwrap();
        drop(conn1);

        let conn2 = open(path_str).unwrap();
        let version: i64 = conn2
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 9);
    }

    #[test]
    fn open_replaces_buggy_pages_update_trigger_for_quarantined_rows() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
        let path_str = db_path.to_str().unwrap();

        let conn = open(path_str).unwrap();
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS pages_au;
             CREATE TRIGGER pages_au AFTER UPDATE ON pages BEGIN
                 INSERT INTO page_fts(page_fts, rowid, title, slug, compiled_truth, timeline)
                 VALUES ('delete', old.id, old.title, old.slug, old.compiled_truth, old.timeline);
                 INSERT INTO page_fts(rowid, title, slug, compiled_truth, timeline)
                 SELECT new.id, new.title, new.slug, new.compiled_truth, new.timeline
                 WHERE new.quarantined_at IS NULL;
             END;",
        )
        .unwrap();
        drop(conn);

        let conn = open(path_str).unwrap();
        let collection_id: i64 = conn
            .query_row(
                "SELECT id FROM collections ORDER BY id LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO pages
                 (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
             VALUES (?1, 'notes/quarantined', ?2, 'concept', 'Quarantined', '', 'before restore', '', '{}', 'notes', '', 1)",
            params![collection_id, uuid::Uuid::now_v7().to_string()],
        )
        .unwrap();
        let page_id = conn.last_insert_rowid();

        conn.execute(
            "UPDATE pages
             SET quarantined_at = '2026-04-25T00:00:00Z'
             WHERE id = ?1",
            [page_id],
        )
        .unwrap();

        conn.execute(
            "UPDATE pages
             SET slug = 'notes/restored',
                 title = 'Restored',
                 compiled_truth = 'after restore',
                 quarantined_at = NULL
             WHERE id = ?1",
            [page_id],
        )
        .unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM page_fts
                 WHERE page_fts MATCH 'after'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn open_backfills_raw_import_content_hash_for_existing_rows() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
        let path_str = db_path.to_str().unwrap();

        let conn = open(path_str).unwrap();
        conn.execute(
            "INSERT INTO pages (slug, uuid, type, title)
             VALUES ('notes/hash-test', ?1, 'concept', 'notes/hash-test')",
            [crate::core::page_uuid::generate_uuid_v7()],
        )
        .unwrap();
        let page_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE slug = 'notes/hash-test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO raw_imports (page_id, import_id, is_active, content_hash, raw_bytes, file_path)
             VALUES (?1, 'import-1', 1, ?2, ?3, 'notes/hash-test.md')",
            params![
                page_id,
                crate::core::raw_imports::content_hash_hex(b"hello"),
                b"hello"
            ],
        )
        .unwrap();
        drop(conn);

        let conn = Connection::open(path_str).unwrap();
        conn.execute_batch(
            "DROP INDEX IF EXISTS idx_raw_imports_content_hash;
             ALTER TABLE raw_imports RENAME TO raw_imports_old;
             CREATE TABLE raw_imports (
                 id         INTEGER PRIMARY KEY AUTOINCREMENT,
                 page_id    INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
                 import_id  TEXT    NOT NULL,
                 is_active  INTEGER NOT NULL DEFAULT 1,
                 raw_bytes  BLOB    NOT NULL,
                 file_path  TEXT    NOT NULL,
                 created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 UNIQUE(page_id, import_id)
             );
             INSERT INTO raw_imports (id, page_id, import_id, is_active, raw_bytes, file_path, created_at)
             SELECT id, page_id, import_id, is_active, raw_bytes, file_path, created_at
             FROM raw_imports_old;
             DROP TABLE raw_imports_old;
             CREATE INDEX idx_raw_imports_page ON raw_imports(page_id);
             CREATE INDEX idx_raw_imports_active ON raw_imports(page_id, is_active)
                 WHERE is_active = 1;",
        )
        .unwrap();
        drop(conn);

        let conn = open(path_str).unwrap();
        let content_hash: String = conn
            .query_row(
                "SELECT content_hash FROM raw_imports WHERE page_id = ?1",
                [page_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            content_hash,
            crate::core::raw_imports::content_hash_hex(b"hello")
        );
    }

    #[test]
    fn open_rebuilds_legacy_pages_unique_constraint_for_namespaces() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
        let path_str = db_path.to_str().unwrap();

        let conn = open(path_str).unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = OFF;
             DROP TRIGGER IF EXISTS pages_ai;
             DROP TRIGGER IF EXISTS pages_ad;
             DROP TRIGGER IF EXISTS pages_au;
             DROP TABLE pages;
             CREATE TABLE pages (
                 id              INTEGER PRIMARY KEY AUTOINCREMENT,
                 collection_id   INTEGER NOT NULL DEFAULT 1 REFERENCES collections(id) ON DELETE CASCADE,
                 slug            TEXT    NOT NULL,
                 uuid            TEXT    DEFAULT NULL,
                 type            TEXT    NOT NULL,
                 title           TEXT    NOT NULL,
                 summary         TEXT    NOT NULL DEFAULT '',
                 compiled_truth  TEXT    NOT NULL DEFAULT '',
                 timeline        TEXT    NOT NULL DEFAULT '',
                 frontmatter     TEXT    NOT NULL DEFAULT '{}',
                 wing            TEXT    NOT NULL DEFAULT '',
                 room            TEXT    NOT NULL DEFAULT '',
                 superseded_by   INTEGER DEFAULT NULL REFERENCES pages(id),
                 version         INTEGER NOT NULL DEFAULT 1,
                 quarantined_at  TEXT    DEFAULT NULL,
                 created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 updated_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 truth_updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 timeline_updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 UNIQUE(collection_id, slug)
             );
             INSERT INTO pages
                 (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
             VALUES
                 (1, 'notes/same-slug', '018f0000-0000-7000-8000-000000000001', 'concept', 'Global', '', '', '', '{}', 'notes', '', 1);
             PRAGMA foreign_keys = ON;",
        )
        .unwrap();
        drop(conn);

        let conn = open(path_str).unwrap();
        conn.execute(
            "INSERT INTO pages
                 (collection_id, namespace, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
             VALUES
                 (1, 'session-a', 'notes/same-slug', ?1, 'concept', 'Namespaced', '', '', '', '{}', 'notes', '', 1)",
            [uuid::Uuid::now_v7().to_string()],
        )
        .unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pages WHERE collection_id = 1 AND slug = 'notes/same-slug'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
        assert!(!pages_needs_namespace_unique_rebuild(&conn).unwrap());

        let duplicate_index_exists = conn
            .query_row(
                "SELECT 1
                 FROM sqlite_master
                 WHERE type = 'index'
                   AND name = 'idx_pages_collection_namespace_slug'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .unwrap()
            .is_some();
        assert!(!duplicate_index_exists);
    }

    #[test]
    fn open_with_model_rejects_v8_database_before_creating_v9_tables() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("legacy.db");
        seed_existing_db(&db_path, 8);

        let err = open_with_model(db_path.to_str().unwrap(), &default_model())
            .expect_err("legacy database should be refused");

        assert!(matches!(err, DbError::Schema { .. }));
        assert!(err.to_string().contains("Found version 8, expected 9"));

        let conn = Connection::open(&db_path).unwrap();
        assert!(!table_exists(&conn, "collections").unwrap());
        let stored_version: String = conn
            .query_row(
                "SELECT value FROM quaid_config WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_version, "8");
    }

    #[test]
    fn init_rejects_v8_database_before_creating_v9_tables() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("legacy.db");
        seed_existing_db(&db_path, 8);

        let err = init(db_path.to_str().unwrap(), &default_model())
            .expect_err("legacy database should be refused");

        assert!(matches!(err, DbError::Schema { .. }));

        let conn = Connection::open(&db_path).unwrap();
        assert!(!table_exists(&conn, "collections").unwrap());
        let config_version: String = conn
            .query_row(
                "SELECT value FROM config WHERE key = 'version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(config_version, "8");
    }

    #[test]
    fn open_connection_seeds_config_version_to_9_for_partial_v9_databases() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("partial-v8.db");
        let conn = open_connection(db_path.to_str().unwrap()).unwrap();
        let config_version: String = conn
            .query_row(
                "SELECT value FROM config WHERE key = 'version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(config_version, "9");
        drop(conn);

        assert!(
            preflight_existing_schema(db_path.to_str().unwrap()).is_ok(),
            "freshly seeded v9 DDL should not be misclassified as legacy before quaid_config is written"
        );
    }

    #[test]
    fn open_with_model_recovers_crash_partial_v9_bootstrap() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("partial-v8.db");
        let conn = open_connection(db_path.to_str().unwrap()).unwrap();
        drop(conn);

        let opened = open_with_model(db_path.to_str().unwrap(), &default_model())
            .expect("crash-partial fresh db should reopen cleanly");
        let stored = read_quaid_config(&opened.conn).unwrap().unwrap();

        assert_eq!(stored.model_alias, "small");
        assert_eq!(stored.schema_version, 9);
    }

    #[cfg(feature = "online-model")]
    #[test]
    fn open_with_model_recovers_crash_partial_v9_bootstrap_from_registry_model() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("partial-v8-registry.db");
        let conn = open_connection(db_path.to_str().unwrap()).unwrap();
        let large_model = hydrate_model_config(&resolve_model("large")).unwrap();
        ensure_embedding_model_registry(&conn, &large_model).unwrap();
        drop(conn);

        let opened = open_with_model(db_path.to_str().unwrap(), &resolve_model("large"))
            .expect("registry-seeded crash-partial db should reopen with the registered model");

        assert_eq!(opened.effective_model.model_id, "BAAI/bge-large-en-v1.5");
        assert_eq!(opened.effective_model.embedding_dim, 1024);
        let stored = read_quaid_config(&opened.conn).unwrap().unwrap();
        assert_eq!(stored.model_alias, "large");
    }

    #[test]
    fn init_recovers_crash_partial_v9_bootstrap() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("partial-v8-init.db");
        let conn = open_connection(db_path.to_str().unwrap()).unwrap();
        drop(conn);

        let conn = init(db_path.to_str().unwrap(), &default_model())
            .expect("init should complete a crash-partial fresh bootstrap");
        let stored = read_quaid_config(&conn).unwrap().unwrap();

        assert_eq!(stored.model_alias, "small");
        assert_eq!(stored.schema_version, 9);
    }

    #[test]
    fn recover_crash_partial_fresh_db_returns_none_after_pages_exist() {
        let conn = open_connection(":memory:").unwrap();
        conn.execute(
            "INSERT INTO pages
                 (slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
             VALUES ('notes/live', ?1, 'concept', 'Live', '', 'truth', '', '{}', 'notes', '', 1)",
            [uuid::Uuid::now_v7().to_string()],
        )
        .unwrap();

        let recovered =
            recover_crash_partial_fresh_db(&conn, &default_model(), "memory.db").unwrap();

        assert!(recovered.is_none());
    }

    #[test]
    fn recover_crash_partial_fresh_db_rejects_registry_model_mismatches() {
        let conn = open_connection(":memory:").unwrap();
        ensure_embedding_model_registry(&conn, &resolve_model("base")).unwrap();

        let err = recover_crash_partial_fresh_db(&conn, &default_model(), "memory.db").unwrap_err();

        assert!(matches!(err, DbError::ModelMismatch { .. }));
    }

    #[test]
    fn read_bootstrap_registry_model_maps_standard_aliases() {
        let conn = open_connection(":memory:").unwrap();
        ensure_embedding_model_registry(&conn, &resolve_model("m3")).unwrap();

        let config = read_bootstrap_registry_model(&conn).unwrap().unwrap();

        assert_eq!(config.model_alias, "m3");
        assert_eq!(config.model_id, "BAAI/bge-m3");
        assert_eq!(config.embedding_dim, 1024);
    }

    #[test]
    fn model_alias_for_model_id_returns_custom_for_unknown_models() {
        assert_eq!(model_alias_for_model_id("org/custom-model"), "custom");
    }

    #[test]
    fn compact_checkpoints_wal() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
        let conn = open(db_path.to_str().unwrap()).unwrap();
        assert!(compact(&conn).is_ok());
    }

    #[test]
    fn open_seeds_default_embedding_model() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
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
    fn persist_model_metadata_writes_registry_quaid_config_and_legacy_config() {
        let conn = open_connection(":memory:").unwrap();

        persist_model_metadata(&conn, &resolve_model("base")).unwrap();

        let stored = read_quaid_config(&conn).unwrap().unwrap();
        let active_model: String = conn
            .query_row(
                "SELECT name FROM embedding_models WHERE active = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let legacy_model: String = conn
            .query_row(
                "SELECT value FROM config WHERE key = 'embedding_model'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(stored.model_alias, "base");
        assert_eq!(stored.embedding_dim, 768);
        assert_eq!(active_model, "BAAI/bge-base-en-v1.5");
        assert_eq!(legacy_model, "BAAI/bge-base-en-v1.5");
    }

    #[test]
    fn is_bootstrap_fresh_db_returns_false_when_inactive_embedding_model_exists() {
        let conn = open_connection(":memory:").unwrap();
        ensure_embedding_model_registry(&conn, &default_model()).unwrap();
        conn.execute("UPDATE embedding_models SET active = 0", [])
            .unwrap();

        assert!(!is_bootstrap_fresh_db(&conn).unwrap());
    }

    #[test]
    fn model_alias_for_model_id_maps_standard_ids() {
        assert_eq!(model_alias_for_model_id("BAAI/bge-small-en-v1.5"), "small");
        assert_eq!(model_alias_for_model_id("BAAI/bge-base-en-v1.5"), "base");
        assert_eq!(model_alias_for_model_id("BAAI/bge-large-en-v1.5"), "large");
    }

    #[test]
    fn quaid_config_to_model_config_restores_pinned_hashes_for_standard_aliases() {
        let config = QuaidConfig {
            model_id: "BAAI/bge-large-en-v1.5".to_owned(),
            model_alias: "large".to_owned(),
            embedding_dim: 1024,
            schema_version: 4,
        };

        let model = config.to_model_config();

        assert_eq!(model.alias, "large");
        assert_eq!(model.model_id, "BAAI/bge-large-en-v1.5");
        assert_eq!(model.embedding_dim, 1024);
        assert!(model.sha256_hashes.is_some());
    }

    #[test]
    fn quaid_config_to_model_config_preserves_custom_model_values() {
        let config = QuaidConfig {
            model_id: "org/custom-model".to_owned(),
            model_alias: "custom".to_owned(),
            embedding_dim: 1536,
            schema_version: 4,
        };

        let model = config.to_model_config();

        assert_eq!(model.alias, "custom");
        assert_eq!(model.model_id, "org/custom-model");
        assert_eq!(model.embedding_dim, 1536);
        assert!(model.sha256_hashes.is_none());
    }

    #[test]
    fn quaid_config_from_model_copies_runtime_metadata() {
        let config = QuaidConfig::from_model(&resolve_model("large"));

        assert_eq!(config.model_id, "BAAI/bge-large-en-v1.5");
        assert_eq!(config.model_alias, "large");
        assert_eq!(config.embedding_dim, 1024);
        assert_eq!(config.schema_version, 9);
    }

    #[test]
    fn quaid_config_roundtrip_preserves_values() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
        let opened = open_with_model(db_path.to_str().unwrap(), &default_model()).unwrap();

        let config = read_quaid_config(&opened.conn).unwrap().unwrap();
        assert_eq!(
            config,
            QuaidConfig {
                model_id: "BAAI/bge-small-en-v1.5".to_owned(),
                model_alias: "small".to_owned(),
                embedding_dim: 384,
                schema_version: 9,
            }
        );
    }

    #[test]
    fn empty_quaid_config_reads_as_missing() {
        let conn = open(":memory:").unwrap();
        conn.execute("DELETE FROM quaid_config", []).unwrap();

        let config = read_quaid_config(&conn).unwrap();
        assert!(config.is_none());
    }

    #[test]
    fn incomplete_quaid_config_returns_schema_error() {
        let conn = open(":memory:").unwrap();
        conn.execute("DELETE FROM quaid_config", []).unwrap();
        conn.execute(
            "INSERT INTO quaid_config (key, value) VALUES ('model_id', 'BAAI/bge-small-en-v1.5')",
            [],
        )
        .unwrap();

        let err = read_quaid_config(&conn).unwrap_err();
        assert!(matches!(err, DbError::Schema { .. }));
    }

    #[test]
    fn read_quaid_config_rejects_non_integer_embedding_dimensions() {
        let conn = open_connection(":memory:").unwrap();
        write_quaid_config(
            &conn,
            &QuaidConfig {
                model_id: "BAAI/bge-small-en-v1.5".to_owned(),
                model_alias: "small".to_owned(),
                embedding_dim: 384,
                schema_version: 4,
            },
        )
        .unwrap();
        conn.execute(
            "UPDATE quaid_config SET value = 'not-a-number' WHERE key = 'embedding_dim'",
            [],
        )
        .unwrap();

        let err = read_quaid_config(&conn).unwrap_err();

        assert!(matches!(err, DbError::Schema { .. }));
    }

    #[test]
    fn read_quaid_config_rejects_non_integer_schema_versions() {
        let conn = open_connection(":memory:").unwrap();
        write_quaid_config(
            &conn,
            &QuaidConfig {
                model_id: "BAAI/bge-small-en-v1.5".to_owned(),
                model_alias: "small".to_owned(),
                embedding_dim: 384,
                schema_version: 4,
            },
        )
        .unwrap();
        conn.execute(
            "UPDATE quaid_config SET value = 'not-a-number' WHERE key = 'schema_version'",
            [],
        )
        .unwrap();

        let err = read_quaid_config(&conn).unwrap_err();

        assert!(matches!(err, DbError::Schema { .. }));
    }

    #[test]
    fn missing_quaid_config_requires_reinit_once_pages_exist() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("test_memory.db");
        let opened = open_with_model(db_path.to_str().unwrap(), &default_model()).unwrap();
        let collection_id: i64 = opened
            .conn
            .query_row(
                "SELECT id FROM collections WHERE name = 'default'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        opened
            .conn
            .execute(
                "INSERT INTO pages
                     (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
                 VALUES (?1, 'notes/live', ?2, 'concept', 'Live', '', 'truth', '', '{}', 'notes', '', 1)",
                params![collection_id, uuid::Uuid::now_v7().to_string()],
            )
            .unwrap();
        opened.conn.execute("DELETE FROM quaid_config", []).unwrap();
        drop(opened);

        let reopened = open_with_model(db_path.to_str().unwrap(), &default_model());
        assert!(matches!(reopened, Err(DbError::Schema { .. })));
    }

    #[test]
    fn mismatch_detection_returns_model_mismatch_error() {
        let stored = QuaidConfig {
            model_id: "BAAI/bge-small-en-v1.5".to_owned(),
            model_alias: "small".to_owned(),
            embedding_dim: 384,
            schema_version: 4,
        };
        let requested = resolve_model("large");
        let message = format_model_mismatch(&stored, &requested, "/tmp/test/memory.db");

        let err = DbError::ModelMismatch { message };
        let DbError::ModelMismatch { message } = &err else {
            unreachable!()
        };
        assert!(message.contains("rm /tmp/test/memory.db && quaid init"));
    }

    #[test]
    fn mismatch_detection_uses_custom_model_id_in_recovery_hint() {
        let stored = QuaidConfig {
            model_id: "org/custom-model".to_owned(),
            model_alias: "custom".to_owned(),
            embedding_dim: 1536,
            schema_version: 4,
        };
        let requested = resolve_model("large");
        let message = format_model_mismatch(&stored, &requested, "memory.db");

        assert!(message.contains("QUAID_MODEL=org/custom-model quaid <command>"));
    }

    #[cfg(feature = "online-model")]
    #[test]
    fn init_with_small_then_open_with_large_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("memory.db");

        init(db_path.to_str().unwrap(), &resolve_model("small")).unwrap();
        let err = open_with_model(db_path.to_str().unwrap(), &resolve_model("large")).unwrap_err();

        assert!(matches!(err, DbError::ModelMismatch { .. }));
    }

    #[cfg(feature = "online-model")]
    #[test]
    fn init_with_large_then_open_with_large_succeeds() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("memory.db");

        let init_opened =
            open_with_model(db_path.to_str().unwrap(), &resolve_model("large")).unwrap();
        drop(init_opened);

        let reopened = open_with_model(db_path.to_str().unwrap(), &resolve_model("large")).unwrap();
        let stored = read_quaid_config(&reopened.conn).unwrap().unwrap();
        assert_eq!(stored.model_alias, "large");
        assert_eq!(stored.embedding_dim, 1024);
    }
}
