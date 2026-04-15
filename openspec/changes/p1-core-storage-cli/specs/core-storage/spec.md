## ADDED Requirements

### Requirement: Database initialisation
The system SHALL create a new SQLite database at the specified path, apply the full v4
DDL from `src/schema.sql` via `include_str!`, enable WAL journal mode, enforce foreign
keys, and load the sqlite-vec extension. The database SHALL be ready for read and write
after `open()` returns.

#### Scenario: First-time init on empty path
- **WHEN** `db::open("/path/to/new/brain.db")` is called and the file does not exist
- **THEN** SQLite creates the file, applies all DDL from `schema.sql`, sets `PRAGMA journal_mode = WAL`, sets `PRAGMA foreign_keys = ON`, loads sqlite-vec, and returns `Ok(Connection)`

#### Scenario: Re-open existing database
- **WHEN** `db::open("/path/to/existing/brain.db")` is called and the file exists
- **THEN** the function returns `Ok(Connection)` without re-running DDL (all `CREATE TABLE IF NOT EXISTS` guards fire but are no-ops)

#### Scenario: Path directory does not exist
- **WHEN** `db::open("/nonexistent/dir/brain.db")` is called
- **THEN** the function returns `Err(DbError::PathNotFound)` and the file is not created

### Requirement: v4 schema completeness
The database schema SHALL include all tables defined in `src/schema.sql` v4: `pages`,
`page_fts` (FTS5 virtual table), `page_embeddings_vec_384` (vec0 virtual table),
`page_embeddings`, `links`, `assertions`, `knowledge_gaps`, `ingest_log`, `config`,
`raw_data`, and all associated indexes and triggers.

#### Scenario: Schema applied on init
- **WHEN** a new database is opened
- **THEN** `SELECT name FROM sqlite_master WHERE type = 'table'` returns at minimum: `pages`, `page_embeddings`, `links`, `assertions`, `knowledge_gaps`, `ingest_log`, `config`

#### Scenario: PRAGMA user_version
- **WHEN** a database is opened
- **THEN** `PRAGMA user_version` returns `4`

### Requirement: Optimistic Concurrency Control on writes
All write operations on the `pages` table SHALL enforce OCC via the `version` column.
The write path SHALL use a compare-and-swap UPDATE that increments `version` atomically.
If the expected version does not match the stored version, the write SHALL be rejected
with a conflict error containing the current stored version.

#### Scenario: First write to new page (create)
- **WHEN** a page is inserted with `version = 1` for the first time
- **THEN** the row is created with `version = 1` and the operation succeeds

#### Scenario: Successful update with correct expected_version
- **WHEN** `put(slug, content, expected_version = 1)` is called and the stored version is `1`
- **THEN** the UPDATE executes with `WHERE slug = ? AND version = 1`, sets `version = 2`, and returns `Ok(2)`

#### Scenario: Conflict on stale expected_version
- **WHEN** `put(slug, content, expected_version = 1)` is called and the stored version is `2`
- **THEN** the UPDATE affects zero rows and returns `Err(OccError::Conflict { current_version: 2 })`

#### Scenario: Unconditional write (no expected_version)
- **WHEN** `put(slug, content, expected_version = None)` is called
- **THEN** the page is upserted without version check and the resulting version is returned

### Requirement: Page version exposed on read
Every page read SHALL include the current `version` field so callers can pass it back
for subsequent writes.

#### Scenario: Get returns version
- **WHEN** `get(slug)` is called for an existing page
- **THEN** the returned `Page` struct has `version` set to the current stored value

### Requirement: WAL checkpoint via compact
The system SHALL expose a `compact(conn)` function that checkpoints the WAL file back
into the main database file, reducing the database to a single transportable file.

#### Scenario: Compact on a live database
- **WHEN** `compact(conn)` is called
- **THEN** `PRAGMA wal_checkpoint(TRUNCATE)` executes and returns `Ok(())`
