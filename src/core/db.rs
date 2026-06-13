//! SQLite connection setup, schema bootstrap, and `sqlite-vec` extension
//! loading. Handles WAL configuration, the embedding-model registry, schema
//! version checks, and crash-partial bootstrap recovery so the rest of the
//! crate can assume an open `Connection` is at the current schema version.
//!
//! Plain opens stay fail-closed on schema-version mismatches; the explicit
//! `quaid migrate` command drives `migrate_database`, which walks the
//! versioned migration ladder (`MIGRATIONS`) with a pre-migration backup,
//! per-step transactions, and post-migration integrity checks.
//!
//! See also: `inference` for the `ModelConfig` values persisted here, `types`
//! for `DbError`, and `migrate` for export/import round-trips.

#![expect(
    clippy::unwrap_used,
    reason = "addressed in remove-production-panic-paths"
)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};

use super::inference::{
    coerce_model_for_build, configure_runtime_model, default_model, hydrate_model_config,
    ModelConfig,
};
use super::types::DbError;

static SQLITE_VEC_INIT: Once = Once::new();
const SCHEMA_VERSION: i64 = 10;
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

/// Result of [`open_with_model`]: the live SQLite connection paired with the
/// model configuration that was actually selected (which may differ from the
/// requested one when the build channel forces a fallback).
pub struct OpenDb {
    /// Underlying SQLite connection with WAL, sqlite-vec, and schema applied.
    pub conn: Connection,
    /// Embedding model the database is currently configured to use.
    pub effective_model: ModelConfig,
}

impl std::fmt::Debug for OpenDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenDb")
            .field("effective_model", &self.effective_model)
            .finish_non_exhaustive()
    }
}

/// Persisted database-level configuration: the embedding model the database
/// was initialized with and the schema version it expects. Stored in the
/// `quaid_config` table and validated on every open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuaidConfig {
    /// HuggingFace id of the embedding model (`<org>/<name>`).
    pub model_id: String,
    /// Short alias for the model (`small`, `base`, `large`, `m3`, or `custom`).
    pub model_alias: String,
    /// Output dimensionality of the embedding model.
    pub embedding_dim: usize,
    /// Schema version this database was created with.
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
    SQLITE_VEC_INIT.call_once(|| {
        // FFI registration of sqlite-vec's auto-extension entry point — the
        // C calling convention is fixed by sqlite3_auto_extension's contract.
        #[expect(
            unsafe_code,
            reason = "sqlite-vec exposes a C entry point we must register via sqlite3_auto_extension; transmute reshapes its function pointer to the SQLite-expected signature"
        )]
        unsafe {
            let init_fn = std::mem::transmute::<
                *const (),
                unsafe extern "C" fn(
                    *mut rusqlite::ffi::sqlite3,
                    *mut *const std::ffi::c_char,
                    *const rusqlite::ffi::sqlite3_api_routines,
                ) -> std::ffi::c_int,
            >(sqlite_vec::sqlite3_vec_init as *const ());
            rusqlite::ffi::sqlite3_auto_extension(Some(init_fn));
        }
    });
}

/// Opens the database at `path` with the default embedding model, applying
/// schema migrations and registering `sqlite-vec` along the way.
pub fn open(path: &str) -> Result<Connection, DbError> {
    open_with_model(path, &default_model()).map(|opened| opened.conn)
}

/// Opens an **already-initialized** database at `path` for runtime use:
/// background workers, watcher callbacks, supervisor ticks, IPC handlers,
/// and other short-lived "fresh connection" sites.
///
/// Unlike [`open`], this performs no DDL, no bootstrap, and no filesystem
/// side effects — the database file must already exist and have been
/// initialized via [`init`]/[`open`] (e.g. `quaid init`). It registers
/// `sqlite-vec`, applies the standard 5s busy timeout, and enables
/// `foreign_keys`, so runtime connections behave identically to [`open`]ed
/// ones under write contention instead of failing instantly with
/// `SQLITE_BUSY`.
///
/// A cheap `PRAGMA user_version` guard rejects files that were never
/// bootstrapped (including `:memory:`, which is always a fresh empty
/// database on a new connection and therefore never valid here).
pub fn open_runtime<P: AsRef<Path>>(path: P) -> Result<Connection, DbError> {
    let db_path = path.as_ref();
    if db_path.as_os_str() != ":memory:" && !db_path.exists() {
        return Err(DbError::PathNotFound {
            path: db_path.display().to_string(),
        });
    }

    ensure_sqlite_vec();
    let conn = Connection::open(db_path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;

    let user_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if user_version != SCHEMA_VERSION {
        return Err(DbError::Schema {
            message: format!(
                "open_runtime requires an initialized database (found user_version {user_version}, expected {SCHEMA_VERSION}) at {}; run `quaid init` or open it via db::open first",
                db_path.display()
            ),
        });
    }

    Ok(conn)
}

/// Runs `action` inside a `BEGIN IMMEDIATE` transaction on `conn`,
/// committing on success and rolling back on failure.
///
/// `SQLITE_BUSY` on COMMIT does **not** auto-rollback, so a failed commit
/// would otherwise leave the transaction open and wedge every subsequent
/// `BEGIN IMMEDIATE` on a shared connection with "cannot start a
/// transaction within a transaction". The explicit ROLLBACK on the
/// commit-error path restores the connection to autocommit; the rollback
/// result is intentionally ignored because the commit error is the one the
/// caller must see (and ROLLBACK after a failed COMMIT can itself report
/// that no transaction is active).
pub fn with_immediate_transaction<T, E>(
    conn: &Connection,
    action: impl FnOnce(&Connection) -> Result<T, E>,
) -> Result<T, E>
where
    E: From<rusqlite::Error>,
{
    conn.execute_batch("BEGIN IMMEDIATE TRANSACTION")?;
    match action(conn) {
        Ok(value) => match conn.execute_batch("COMMIT TRANSACTION") {
            Ok(()) => Ok(value),
            Err(commit_error) => {
                let _ = conn.execute_batch("ROLLBACK TRANSACTION");
                Err(E::from(commit_error))
            }
        },
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK TRANSACTION");
            Err(error)
        }
    }
}

/// Returns the conventional `~/.quaid/memory.db` path, falling back to
/// `memory.db` in the current directory when no home directory is available.
pub fn default_db_path() -> std::path::PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".quaid").join("memory.db"))
        .unwrap_or_else(|| std::path::PathBuf::from("memory.db"))
}

/// String form of [`default_db_path`] for use in error messages.
pub fn default_db_path_string() -> String {
    default_db_path().display().to_string()
}

/// Returns the conventional first-run collection root at `~/.quaid/vault`.
pub fn default_collection_root_path() -> Result<PathBuf, DbError> {
    dirs::home_dir()
        .map(|home| home.join(".quaid").join("vault"))
        .ok_or_else(|| DbError::Schema {
            message: "could not resolve home directory for default collection root".to_owned(),
        })
}

/// Opens the database at `path` and ensures it is configured for
/// `requested_model`, returning the effective model along with the connection.
/// Rejects schema-version or model-id mismatches against any prior init.
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
            // Check schema version — refuse to open any mismatched schema version
            if stored.schema_version != SCHEMA_VERSION {
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

/// Initializes a new database at `path` with `requested_model`, or re-opens
/// an existing one. Used by `quaid init` and the test harness; recovers from
/// crash-partial bootstraps and persists the model metadata on first init.
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

    if schema_version != SCHEMA_VERSION {
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
    apply_current_version_maintenance(&conn)?;
    set_version(&conn)?;
    ensure_default_collection(&conn)?;

    Ok(conn)
}

/// One rung of the versioned migration ladder: upgrades a database whose
/// stored schema version is `target - 1` to `target`. Step functions run
/// inside a transaction opened by [`migrate_database`] (with foreign keys
/// disabled for table rebuilds) and must not begin or commit transactions
/// themselves; the runner bumps the stored schema version after each step.
type MigrationStep = fn(&Connection) -> Result<(), DbError>;

/// Versioned migration ladder, keyed by target schema version. Plain `open`
/// stays fail-closed on any schema-version mismatch; only the explicit
/// `quaid migrate` command ([`migrate_database`]) walks these rungs. New DDL
/// changes must be added here as a new rung with a `SCHEMA_VERSION` bump,
/// not as silent open-time patches.
const MIGRATIONS: &[(i64, MigrationStep)] = &[(10, migrate_v9_to_v10_links_graph)];

/// Idempotent same-version maintenance: the registry's current-version step,
/// applied on every open and at the end of every `migrate_database` run.
///
/// This folds the formerly scattered unversioned `ensure_*` patches —
/// `ensure_pages_update_trigger_handles_quarantine`, `ensure_namespace_schema`,
/// `ensure_collection_owner_columns`, `ensure_serve_session_columns`,
/// `ensure_collection_name_guards`, and `ensure_raw_import_hash_schema` —
/// into one place so current-version databases converge on one shape and
/// future DDL changes land as new [`MIGRATIONS`] rungs instead of silent
/// per-release drift.
fn apply_current_version_maintenance(conn: &Connection) -> Result<(), DbError> {
    ensure_pages_update_trigger_handles_quarantine(conn)?;
    ensure_namespace_schema(conn)?;
    ensure_collection_owner_columns(conn)?;
    ensure_serve_session_columns(conn)?;
    ensure_collection_name_guards(conn)?;
    ensure_raw_import_hash_schema(conn)?;
    ensure_file_state_uuid_cache_schema(conn)?;
    Ok(())
}

/// v9 → v10: the knowledge-graph layer on `links`.
///
/// Implements exactly the `src/schema.sql` delta between v0.21.x (schema v9)
/// and v0.22.x (schema v10):
/// - extends the `source_kind` CHECK with `'frontmatter'` and
///   `'entity_pattern'` and adds `edge_weight REAL NOT NULL DEFAULT 1.0` via
///   the documented 12-step table rebuild (CHECK constraints cannot be
///   altered in place);
/// - dedupes derived edges (keeping the oldest row per
///   `(from, to, relationship, source_kind)`; the next sync refreshes its
///   temporal fields anyway), then creates the partial unique index
///   `idx_links_unique_derived_edge`;
/// - seeds the v10 graph config defaults.
fn migrate_v9_to_v10_links_graph(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        "DELETE FROM links
          WHERE source_kind IN ('wiki_link', 'frontmatter', 'entity_pattern')
            AND id NOT IN (
                SELECT MIN(id)
                  FROM links
                 WHERE source_kind IN ('wiki_link', 'frontmatter', 'entity_pattern')
                 GROUP BY from_page_id, to_page_id, relationship, source_kind
            );

         CREATE TABLE links_new (
             id              INTEGER PRIMARY KEY AUTOINCREMENT,
             from_page_id    INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
             to_page_id      INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
             relationship    TEXT    NOT NULL DEFAULT 'related',
             context         TEXT    NOT NULL DEFAULT '',
             source_kind     TEXT    NOT NULL DEFAULT 'programmatic' CHECK(source_kind IN ('wiki_link', 'programmatic', 'frontmatter', 'entity_pattern')),
             edge_weight     REAL    NOT NULL DEFAULT 1.0,
             valid_from      TEXT    DEFAULT NULL,
             valid_until     TEXT    DEFAULT NULL,
             created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
             CHECK (valid_from IS NULL OR valid_until IS NULL OR valid_until >= valid_from)
         );
         INSERT INTO links_new
             (id, from_page_id, to_page_id, relationship, context, source_kind,
              edge_weight, valid_from, valid_until, created_at)
         SELECT id, from_page_id, to_page_id, relationship, context, source_kind,
                1.0, valid_from, valid_until, created_at
           FROM links;
         DROP TABLE links;
         ALTER TABLE links_new RENAME TO links;

         CREATE INDEX IF NOT EXISTS idx_links_from    ON links(from_page_id);
         CREATE INDEX IF NOT EXISTS idx_links_to      ON links(to_page_id);
         CREATE INDEX IF NOT EXISTS idx_links_current ON links(valid_until);
         CREATE INDEX IF NOT EXISTS idx_links_source  ON links(source_kind);
         CREATE UNIQUE INDEX IF NOT EXISTS idx_links_unique_derived_edge
             ON links(from_page_id, to_page_id, relationship, source_kind)
             WHERE source_kind IN ('wiki_link', 'frontmatter', 'entity_pattern');

         INSERT OR IGNORE INTO config (key, value) VALUES
             ('graph_depth',                  '0'),
             ('graph_distance_decay',         '0.5'),
             ('graph_expansion_max',          '50'),
             ('edge_weight_frontmatter',      '1.0'),
             ('edge_weight_entity_pattern',   '0.7'),
             ('edge_weight_wikilink',         '0.5');",
    )?;
    Ok(())
}

/// Result of a [`migrate_database`] run: versions, applied ladder rungs,
/// backup location, and row-count sanity figures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    /// Schema version stored in the database before the run.
    pub from_version: i64,
    /// Schema version after the run (the binary's current schema version).
    pub to_version: i64,
    /// Target versions of the ladder rungs that were applied, in order.
    /// Empty when the database was already current.
    pub steps_applied: Vec<i64>,
    /// Path of the pre-migration `.bak` copy, written before the first rung.
    /// `None` when no rungs were applied.
    pub backup_path: Option<String>,
    /// `pages` row count before the ladder ran.
    pub pages_before: i64,
    /// `pages` row count after the ladder ran (must equal `pages_before`).
    pub pages_after: i64,
    /// `links` row count before the ladder ran.
    pub links_before: i64,
    /// `links` row count after the ladder ran (may shrink: duplicate derived
    /// edges are collapsed by the v9 → v10 rung).
    pub links_after: i64,
}

/// Upgrades the database at `path` to the current schema version by walking
/// the versioned `MIGRATIONS` ladder. Backs the CLI's explicit
/// `quaid migrate` command; plain opens stay fail-closed on mismatches.
///
/// Order of operations:
/// 1. refuse databases newer than this binary or with no registered path;
/// 2. write a `<path>.bak` copy (after a WAL checkpoint) before the first
///    rung;
/// 3. run each rung in its own transaction, bumping
///    `quaid_config.schema_version` and the legacy `config.version` mirror
///    per step, with a `foreign_key_check` after each commit;
/// 4. apply current-version maintenance, then verify
///    `PRAGMA integrity_check` and row-count sanity.
///
/// An already-current database is a no-op (maintenance still runs and no
/// backup is written).
pub fn migrate_database(path: &str) -> Result<MigrationReport, DbError> {
    if path == ":memory:" {
        return Err(DbError::Schema {
            message: "in-memory databases cannot be migrated".to_owned(),
        });
    }
    if !Path::new(path).exists() {
        return Err(DbError::PathNotFound {
            path: path.to_owned(),
        });
    }

    ensure_sqlite_vec();
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(5))?;

    let Some(from_version) = read_existing_schema_version(&conn)? else {
        return Err(DbError::Schema {
            message: format_schema_reinit_message(0, path),
        });
    };
    if from_version > SCHEMA_VERSION {
        return Err(DbError::Schema {
            message: format_schema_reinit_message(from_version, path),
        });
    }

    // Verify a complete ladder exists before touching anything.
    let pending: Vec<(i64, MigrationStep)> = ((from_version + 1)..=SCHEMA_VERSION)
        .map(|target| {
            MIGRATIONS
                .iter()
                .find(|(version, _)| *version == target)
                .copied()
                .ok_or_else(|| DbError::Schema {
                    message: format!(
                        "no migration step is registered for schema version {target}; this database cannot be migrated in place.\n{}",
                        format_schema_reinit_message(from_version, path)
                    ),
                })
        })
        .collect::<Result<_, _>>()?;

    let pages_before = count_table_rows(&conn, "pages")?;
    let links_before = count_table_rows(&conn, "links")?;

    let mut backup_path = None;
    let mut steps_applied = Vec::with_capacity(pending.len());
    if !pending.is_empty() {
        backup_path = Some(write_pre_migration_backup(&conn, path)?);
        for (target, step) in pending {
            apply_migration_step(&conn, target, step)?;
            steps_applied.push(target);
        }
    }

    apply_current_version_maintenance(&conn)?;
    set_version(&conn)?;

    let backup_note = backup_path
        .as_deref()
        .unwrap_or("none (no migration steps were applied)");

    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(DbError::Schema {
            message: format!(
                "integrity_check failed after migration: {integrity}\n  Pre-migration backup: {backup_note}"
            ),
        });
    }

    let pages_after = count_table_rows(&conn, "pages")?;
    let links_after = count_table_rows(&conn, "links")?;
    if pages_after != pages_before || links_after > links_before {
        return Err(DbError::Schema {
            message: format!(
                "row-count sanity check failed after migration (pages {pages_before} -> {pages_after}, links {links_before} -> {links_after})\n  Pre-migration backup: {backup_note}"
            ),
        });
    }

    Ok(MigrationReport {
        from_version,
        to_version: SCHEMA_VERSION,
        steps_applied,
        backup_path,
        pages_before,
        pages_after,
        links_before,
        links_after,
    })
}

fn count_table_rows(conn: &Connection, table: &str) -> Result<i64, DbError> {
    if !table_exists(conn, table)? {
        return Ok(0);
    }
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .map_err(DbError::from)
}

fn write_pre_migration_backup(conn: &Connection, path: &str) -> Result<String, DbError> {
    // Fold the WAL into the main file so the copy is a complete snapshot.
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    let backup = format!("{path}.bak");
    std::fs::copy(path, &backup).map_err(|error| DbError::Schema {
        message: format!("failed to write pre-migration backup at {backup}: {error}"),
    })?;
    Ok(backup)
}

fn apply_migration_step(
    conn: &Connection,
    target: i64,
    step: MigrationStep,
) -> Result<(), DbError> {
    let foreign_keys_enabled: i64 = conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    conn.execute_batch("PRAGMA foreign_keys = OFF;")?;

    let applied: Result<(), DbError> = (|| {
        let tx = conn.unchecked_transaction()?;
        step(&tx)?;
        bump_stored_schema_version(&tx, target)?;
        tx.commit()?;
        Ok(())
    })();

    if foreign_keys_enabled != 0 {
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    }
    applied?;

    let violation: Option<String> = conn
        .query_row("PRAGMA foreign_key_check", [], |row| row.get(0))
        .optional()?;
    if let Some(table) = violation {
        return Err(DbError::Schema {
            message: format!(
                "foreign_key_check failed after migrating to schema version {target} (first violation in table {table})"
            ),
        });
    }
    Ok(())
}

/// Bumps the stored schema version in both places it is written today:
/// `quaid_config.schema_version` (upsert, mirroring `write_quaid_config`)
/// and the legacy `config.version` mirror (`INSERT OR REPLACE`, mirroring
/// `sync_legacy_config`).
fn bump_stored_schema_version(conn: &Connection, version: i64) -> Result<(), DbError> {
    if table_exists(conn, "quaid_config")? {
        conn.execute(
            "INSERT INTO quaid_config (key, value) VALUES ('schema_version', ?1) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [version.to_string()],
        )?;
    }
    if table_exists(conn, "config")? {
        conn.execute(
            "INSERT OR REPLACE INTO config (key, value) VALUES ('version', ?1)",
            [version.to_string()],
        )?;
    }
    Ok(())
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

/// Adds the `file_state.frontmatter_uuid` cache column to databases created
/// before it existed in `schema.sql`. NULL means "not yet cached" so the
/// reconciler's duplicate-uuid scan lazily backfills it on the next pass;
/// no eager backfill is needed here.
fn ensure_file_state_uuid_cache_schema(conn: &Connection) -> Result<(), DbError> {
    if !table_exists(conn, "file_state")? {
        return Ok(());
    }

    let mut stmt = conn.prepare("PRAGMA table_info(file_state)")?;
    let existing_columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<HashSet<_>, _>>()?;

    if !existing_columns.contains("frontmatter_uuid") {
        conn.execute_batch(
            "ALTER TABLE file_state ADD COLUMN frontmatter_uuid TEXT DEFAULT NULL;",
        )?;
    }

    Ok(())
}

/// Ensure a collection with id=1 exists in the database.
///
/// All legacy INSERT INTO pages statements that omit collection_id rely on
/// `DEFAULT 1` routing them to this collection.  Called at every
/// `open_connection()` so test-only in-memory databases are also covered.
///
/// Deliberately performs **no filesystem side effects**: the row is seeded
/// with `root_path = ''` and the on-disk default root (`~/.quaid/vault`) is
/// only provisioned by [`provision_default_collection_root`] — called from
/// `quaid init` and from write-target resolution — whose ON CONFLICT branch
/// heals the empty `root_path` placeholder.
fn ensure_default_collection(conn: &Connection) -> Result<(), DbError> {
    if has_configured_write_target(conn)? {
        return Ok(());
    }

    upsert_default_collection(conn, "")
}

/// Provisions the default collection root (`~/.quaid/vault`) on disk and
/// heals the default collection row when its `root_path` is still the
/// empty-string placeholder seeded at open. Called by `quaid init` and by
/// write-target resolution points just before a write needs a usable root;
/// a no-op when a write target with a non-empty root is already configured.
pub fn provision_default_collection_root(conn: &Connection) -> Result<(), DbError> {
    if has_configured_write_target(conn)? {
        return Ok(());
    }

    let default_root = prepare_default_collection_root()?;
    upsert_default_collection(conn, &default_root)
}

fn upsert_default_collection(conn: &Connection, default_root: &str) -> Result<(), DbError> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE collections SET is_write_target = 0 WHERE is_write_target != 0",
        [],
    )?;
    tx.execute(
        "INSERT INTO collections \
             (id, name, root_path, state, writable, is_write_target, needs_full_sync) \
         VALUES (1, 'default', ?1, 'active', 1, 1, 0) \
         ON CONFLICT(id) DO UPDATE SET \
             root_path = CASE \
                 WHEN trim(collections.root_path) = '' THEN excluded.root_path \
                 ELSE collections.root_path \
             END, \
             state = CASE \
                 WHEN trim(collections.root_path) = '' THEN 'active' \
                 ELSE collections.state \
             END, \
             writable = CASE \
                 WHEN trim(collections.root_path) = '' THEN 1 \
                 ELSE collections.writable \
             END, \
             is_write_target = 1, \
             needs_full_sync = CASE \
                 WHEN trim(collections.root_path) = '' THEN 0 \
                 ELSE collections.needs_full_sync \
             END, \
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
        [default_root],
    )?;
    tx.commit()?;
    Ok(())
}

/// Returns whether a write-target collection with a usable (non-empty)
/// root is configured — the guard both [`ensure_default_collection`] and
/// [`provision_default_collection_root`] use to stay no-ops once a real
/// write target exists.
fn has_configured_write_target(conn: &Connection) -> Result<bool, DbError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM collections
         WHERE is_write_target = 1
           AND trim(root_path) != ''",
        [],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn prepare_default_collection_root() -> Result<String, DbError> {
    let root = default_collection_root_path()?;
    std::fs::create_dir_all(&root).map_err(|error| DbError::Schema {
        message: format!(
            "failed to create default collection root at {}: {error}",
            root.display()
        ),
    })?;
    let resolved = std::fs::canonicalize(&root).map_err(|error| DbError::Schema {
        message: format!(
            "failed to resolve default collection root at {}: {error}",
            root.display()
        ),
    })?;
    Ok(resolved.display().to_string())
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
        "correction_sessions",
        "embedding_jobs",
        "extraction_queue",
        "file_state",
        "import_manifest",
        "knowledge_gaps",
        "links",
        "namespaces",
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

/// Writes the four `quaid_config` keys (`model_id`, `model_alias`,
/// `embedding_dim`, `schema_version`) atomically so a mid-flight crash never
/// leaves a partial config that would silently fall back to the small model.
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

/// Reads the persisted [`QuaidConfig`] back out, returning `Ok(None)` for
/// legacy databases that pre-date the `quaid_config` table and an error for
/// tables that exist but are missing required keys (partial-write corruption).
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

/// Reads a single value out of the legacy `config` key/value table by key.
pub fn read_config_value(conn: &Connection, key: &str) -> Result<Option<String>, DbError> {
    conn.query_row("SELECT value FROM config WHERE key = ?1", [key], |row| {
        row.get(0)
    })
    .optional()
    .map_err(DbError::from)
}

/// Reads a single value out of the legacy `config` table, returning `default`
/// (owned) when the key is absent.
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
    let header = format!(
        "Error: database schema version mismatch.\n  Found version {schema_version}, expected {SCHEMA_VERSION}.\n  Database: {path}"
    );

    if schema_version > SCHEMA_VERSION {
        // Newer-than-current: the data is fine, this binary is just too old.
        return format!(
            "{header}\n  This database was created by a NEWER quaid release than this binary supports.\n  Upgrade the quaid binary to a release that supports schema version {schema_version}.\n  Do NOT run `quaid init` against this database, and back up the file before any downgrade attempt."
        );
    }

    let provenance = if schema_version == 0 {
        // True legacy era: no readable schema version in either the
        // `quaid_config` or the legacy `config` table.
        "This file has no readable schema version: it was created by a legacy release predating versioned Quaid configs, or it is not a Quaid database.".to_owned()
    } else {
        let release_hint = if schema_version == 9 {
            " (schema version 9 shipped with quaid v0.20.x-v0.21.x)"
        } else {
            ""
        };
        format!(
            "This database was created by an older quaid release{release_hint}. This binary does not migrate older schemas automatically."
        )
    };

    format!(
        "{header}\n  {provenance}\n  To migrate without losing data:\n    1. BACK UP / move the old database out of the way first:\n         mv {path} {path}.bak\n       (`quaid init` refuses to run while the old file sits at this path)\n    2. Re-create the store:        quaid init {path}\n    3. If you still have a matching older quaid release, `quaid export` your data\n       with it, then re-ingest the exported markdown:\n         quaid collection add migrated <export-directory>\n         # or ingest files individually with `quaid ingest`"
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

/// Runs a `PRAGMA wal_checkpoint(TRUNCATE)` to reclaim WAL disk space without
/// closing the connection.
pub fn compact(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(())
}

/// Writes the current schema version into SQLite's `user_version` pragma so
/// external tooling can verify compatibility without opening a transaction.
pub fn set_version(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
    Ok(())
}

// reason: white-box; needs `seed_existing_db`, `open_connection`,
// `preflight_existing_schema`, `table_exists`, `pages_needs_namespace_unique_rebuild`,
// `recover_crash_partial_fresh_db`, `read_bootstrap_registry_model`,
// `model_alias_for_model_id`, `ensure_embedding_model_registry`,
// `persist_model_metadata`, `is_bootstrap_fresh_db`, `format_model_mismatch`,
// and the private `QuaidConfig::{from_model, to_model_config}` methods.
// Public-API tests for `open`, `open_with_model`, `init`, `compact`,
// `read_quaid_config`, `write_quaid_config`, and `QuaidConfig` round-trips
// live under `tests/db_*.rs`.
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
    fn open_with_model_rejects_v9_database_before_creating_v10_tables() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("legacy.db");
        seed_existing_db(&db_path, 9);

        let err = open_with_model(db_path.to_str().unwrap(), &default_model())
            .expect_err("legacy database should be refused");

        assert!(matches!(err, DbError::Schema { .. }));
        assert!(err.to_string().contains("Found version 9, expected 10"));

        let conn = Connection::open(&db_path).unwrap();
        assert!(!table_exists(&conn, "collections").unwrap());
        let stored_version: String = conn
            .query_row(
                "SELECT value FROM quaid_config WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_version, "9");
    }

    #[test]
    fn init_rejects_v9_database_before_creating_v10_tables() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("legacy.db");
        seed_existing_db(&db_path, 9);

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
        assert_eq!(config_version, "9");
    }

    #[test]
    fn open_with_model_rejects_future_schema_database_before_creating_v10_tables() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("future.db");
        seed_existing_db(&db_path, 11);

        let err = open_with_model(db_path.to_str().unwrap(), &default_model())
            .expect_err("future schema database should be refused");

        assert!(matches!(err, DbError::Schema { .. }));
        assert!(err.to_string().contains("Found version 11, expected 10"));

        let conn = Connection::open(&db_path).unwrap();
        assert!(!table_exists(&conn, "collections").unwrap());
        let stored_version: String = conn
            .query_row(
                "SELECT value FROM quaid_config WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_version, "11");
    }

    #[test]
    fn init_rejects_future_schema_database_before_creating_v10_tables() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("future.db");
        seed_existing_db(&db_path, 11);

        let err = init(db_path.to_str().unwrap(), &default_model())
            .expect_err("future schema database should be refused");

        assert!(matches!(err, DbError::Schema { .. }));
        assert!(err.to_string().contains("Found version 11, expected 10"));

        let conn = Connection::open(&db_path).unwrap();
        assert!(!table_exists(&conn, "collections").unwrap());
        let config_version: String = conn
            .query_row(
                "SELECT value FROM config WHERE key = 'version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(config_version, "11");
    }

    #[test]
    fn open_connection_seeds_config_version_to_10_for_partial_v10_databases() {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("partial-v10.db");
        let conn = open_connection(db_path.to_str().unwrap()).unwrap();
        let config_version: String = conn
            .query_row(
                "SELECT value FROM config WHERE key = 'version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(config_version, "10");
        drop(conn);

        assert!(
            preflight_existing_schema(db_path.to_str().unwrap()).is_ok(),
            "freshly seeded v10 DDL should not be misclassified as legacy before quaid_config is written"
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
        assert_eq!(stored.schema_version, 10);
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
        assert_eq!(stored.schema_version, 10);
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
        assert_eq!(config.schema_version, 10);
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
}
