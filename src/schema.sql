-- memory.db schema — Quaid v7
-- Embedded in binary via include_str!("schema.sql") in src/core/db.rs
-- Standalone copy for reference and tooling.

PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

-- ============================================================
-- quaid_config: persisted embedding model metadata
-- ============================================================
CREATE TABLE IF NOT EXISTS quaid_config (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;

-- ============================================================
-- collections: named groupings with their own root, ignore patterns
-- ============================================================
CREATE TABLE IF NOT EXISTS collections (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    name                TEXT    NOT NULL UNIQUE,
    root_path           TEXT    NOT NULL,
    state               TEXT    NOT NULL DEFAULT 'active' CHECK(state IN ('active', 'detached', 'restoring')),
    writable            INTEGER NOT NULL DEFAULT 1,
    is_write_target     INTEGER NOT NULL DEFAULT 0,
    ignore_patterns     TEXT    DEFAULT NULL,
    ignore_parse_errors TEXT    DEFAULT NULL,
    needs_full_sync     INTEGER NOT NULL DEFAULT 0,
    last_sync_at        TEXT    DEFAULT NULL,
    active_lease_session_id   TEXT DEFAULT NULL,
    restore_command_id        TEXT DEFAULT NULL,
    restore_lease_session_id  TEXT DEFAULT NULL,
    reload_generation         INTEGER NOT NULL DEFAULT 0,
    watcher_released_session_id TEXT DEFAULT NULL,
    watcher_released_generation INTEGER DEFAULT NULL,
    watcher_released_at         TEXT DEFAULT NULL,
    pending_command_heartbeat_at TEXT DEFAULT NULL,
    pending_root_path           TEXT DEFAULT NULL,
    pending_restore_manifest    TEXT DEFAULT NULL,
    restore_command_pid         INTEGER DEFAULT NULL,
    restore_command_host        TEXT DEFAULT NULL,
    integrity_failed_at         TEXT DEFAULT NULL,
    pending_manifest_incomplete_at TEXT DEFAULT NULL,
    reconcile_halted_at         TEXT DEFAULT NULL,
    reconcile_halt_reason       TEXT DEFAULT NULL,
    created_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_collections_write_target
    ON collections(is_write_target) WHERE is_write_target = 1;
CREATE INDEX IF NOT EXISTS idx_collections_restore_state
    ON collections(state, needs_full_sync, reload_generation);
CREATE INDEX IF NOT EXISTS idx_collections_reconcile_halt
    ON collections(reconcile_halted_at) WHERE reconcile_halted_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS serve_sessions (
    session_id    TEXT PRIMARY KEY,
    pid           INTEGER NOT NULL,
    host          TEXT    NOT NULL,
    started_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    heartbeat_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ipc_path      TEXT    DEFAULT NULL,
    session_type  TEXT    NOT NULL DEFAULT 'serve'
) STRICT;

CREATE INDEX IF NOT EXISTS idx_serve_sessions_heartbeat
    ON serve_sessions(heartbeat_at);

CREATE TABLE IF NOT EXISTS collection_owners (
    collection_id INTEGER PRIMARY KEY REFERENCES collections(id) ON DELETE CASCADE,
    session_id    TEXT NOT NULL REFERENCES serve_sessions(session_id) ON DELETE CASCADE,
    claimed_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
) STRICT;

CREATE INDEX IF NOT EXISTS idx_collection_owners_session
    ON collection_owners(session_id);

-- ============================================================
-- pages: the core content table
-- ============================================================
CREATE TABLE IF NOT EXISTS pages (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    -- DEFAULT 1 routes legacy inserts to the default collection (id=1).
    -- A matching ensure_default_collection() call in db.rs guarantees the row exists.
    collection_id   INTEGER NOT NULL DEFAULT 1 REFERENCES collections(id) ON DELETE CASCADE,
    namespace       TEXT    NOT NULL DEFAULT '',
    slug            TEXT    NOT NULL,
    -- NULL until UUID lifecycle (tasks 5a.*) is fully wired; allows NULL so
    -- legacy INSERT helpers that omit uuid continue to work.
    uuid            TEXT    DEFAULT NULL,
    type            TEXT    NOT NULL,
    -- Valid types: person, company, deal, yc, civic, project, concept, original,
    --              source, media, decision, commitment, action_item
    title           TEXT    NOT NULL,
    summary         TEXT    NOT NULL DEFAULT '',
    compiled_truth  TEXT    NOT NULL DEFAULT '',
    timeline        TEXT    NOT NULL DEFAULT '',
    frontmatter     TEXT    NOT NULL DEFAULT '{}',
    wing            TEXT    NOT NULL DEFAULT '',
    room            TEXT    NOT NULL DEFAULT '',
    version         INTEGER NOT NULL DEFAULT 1,
    quarantined_at  TEXT    DEFAULT NULL,
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    truth_updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    timeline_updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(collection_id, namespace, slug)
);

CREATE INDEX IF NOT EXISTS idx_pages_namespace ON pages(namespace);
-- Partial index: SQLite allows multiple NULLs in unique indexes, but being
-- explicit here avoids confusion when uuid is still unset.
CREATE UNIQUE INDEX IF NOT EXISTS idx_pages_uuid ON pages(uuid) WHERE uuid IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_pages_collection ON pages(collection_id);
CREATE INDEX IF NOT EXISTS idx_pages_type     ON pages(type);
CREATE INDEX IF NOT EXISTS idx_pages_slug     ON pages(collection_id, slug);
CREATE INDEX IF NOT EXISTS idx_pages_updated  ON pages(updated_at);
CREATE INDEX IF NOT EXISTS idx_pages_wing     ON pages(wing);
CREATE INDEX IF NOT EXISTS idx_pages_wing_room ON pages(wing, room);
CREATE INDEX IF NOT EXISTS idx_pages_quarantined ON pages(quarantined_at) WHERE quarantined_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS namespaces (
    id         TEXT PRIMARY KEY,
    ttl_hours  REAL DEFAULT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
) STRICT;

CREATE TABLE IF NOT EXISTS quarantine_exports (
    page_id         INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    quarantined_at  TEXT    NOT NULL,
    exported_at     TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    output_path     TEXT    NOT NULL,
    PRIMARY KEY (page_id, quarantined_at)
);

CREATE INDEX IF NOT EXISTS idx_quarantine_exports_exported
    ON quarantine_exports(exported_at);

-- ============================================================
-- page_fts: full-text search over compiled_truth + timeline
-- ============================================================
CREATE VIRTUAL TABLE IF NOT EXISTS page_fts USING fts5(
    title,
    slug,
    compiled_truth,
    timeline,
    content='pages',
    content_rowid='id',
    tokenize='porter unicode61'
);

CREATE TRIGGER IF NOT EXISTS pages_ai AFTER INSERT ON pages BEGIN
    INSERT INTO page_fts(rowid, title, slug, compiled_truth, timeline)
    SELECT new.id, new.title, new.slug, new.compiled_truth, new.timeline
    WHERE new.quarantined_at IS NULL;
END;

CREATE TRIGGER IF NOT EXISTS pages_ad AFTER DELETE ON pages BEGIN
    INSERT INTO page_fts(page_fts, rowid, title, slug, compiled_truth, timeline)
    SELECT 'delete', old.id, old.title, old.slug, old.compiled_truth, old.timeline
    WHERE old.quarantined_at IS NULL;
END;

CREATE TRIGGER IF NOT EXISTS pages_au AFTER UPDATE ON pages BEGIN
    INSERT INTO page_fts(page_fts, rowid, title, slug, compiled_truth, timeline)
    SELECT 'delete', old.id, old.title, old.slug, old.compiled_truth, old.timeline
    WHERE old.quarantined_at IS NULL;
    INSERT INTO page_fts(rowid, title, slug, compiled_truth, timeline)
    SELECT new.id, new.title, new.slug, new.compiled_truth, new.timeline
    WHERE new.quarantined_at IS NULL;
END;

-- ============================================================
-- embedding_models: registry — one active model at all times
-- ============================================================
CREATE TABLE IF NOT EXISTS embedding_models (
    name        TEXT    PRIMARY KEY,
    dimensions  INTEGER NOT NULL,
    vec_table   TEXT    NOT NULL UNIQUE,
    active      INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- Exactly one active model enforced via partial unique index.
CREATE UNIQUE INDEX IF NOT EXISTS idx_embedding_models_one_active
    ON embedding_models(active) WHERE active = 1;

-- Seed default model at init:
--   INSERT INTO embedding_models (name, dimensions, vec_table, active)
--   VALUES ('BAAI/bge-small-en-v1.5', 384, 'page_embeddings_vec_384', 1);
-- Vec table created dynamically in db.rs:
--   CREATE VIRTUAL TABLE IF NOT EXISTS page_embeddings_vec_384 USING vec0(embedding float[384]);

-- ============================================================
-- page_embeddings: chunk metadata for vector search
-- ============================================================
CREATE TABLE IF NOT EXISTS page_embeddings (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id         INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    model           TEXT    NOT NULL DEFAULT 'BAAI/bge-small-en-v1.5'
                            REFERENCES embedding_models(name),
    vec_rowid       INTEGER NOT NULL,
    chunk_type      TEXT    NOT NULL,   -- 'truth_section' | 'timeline_entry'
    chunk_index     INTEGER NOT NULL,
    chunk_text      TEXT    NOT NULL,
    content_hash    TEXT    NOT NULL,   -- SHA-256 of chunk_text
    token_count     INTEGER NOT NULL,
    heading_path    TEXT    NOT NULL DEFAULT '',
    last_embedded_at TEXT   NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(model, vec_rowid)
);

CREATE INDEX IF NOT EXISTS idx_embeddings_page   ON page_embeddings(page_id);
CREATE INDEX IF NOT EXISTS idx_embeddings_model  ON page_embeddings(model);
CREATE INDEX IF NOT EXISTS idx_embeddings_lookup ON page_embeddings(model, page_id, chunk_index);

-- ============================================================
-- links: typed temporal cross-references between pages
-- ============================================================
CREATE TABLE IF NOT EXISTS links (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    from_page_id    INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    to_page_id      INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    relationship    TEXT    NOT NULL DEFAULT 'related',
    context         TEXT    NOT NULL DEFAULT '',
    source_kind     TEXT    NOT NULL DEFAULT 'programmatic' CHECK(source_kind IN ('wiki_link', 'programmatic')),
    valid_from      TEXT    DEFAULT NULL,
    valid_until     TEXT    DEFAULT NULL,
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    CHECK (valid_from IS NULL OR valid_until IS NULL OR valid_until >= valid_from)
);

CREATE INDEX IF NOT EXISTS idx_links_from    ON links(from_page_id);
CREATE INDEX IF NOT EXISTS idx_links_to      ON links(to_page_id);
CREATE INDEX IF NOT EXISTS idx_links_current ON links(valid_until);
CREATE INDEX IF NOT EXISTS idx_links_source  ON links(source_kind);

-- ============================================================
-- assertions: heuristic contradiction detection
-- ============================================================
CREATE TABLE IF NOT EXISTS assertions (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id         INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    subject         TEXT    NOT NULL,
    predicate       TEXT    NOT NULL,
    object          TEXT    NOT NULL,
    valid_from      TEXT    DEFAULT NULL,
    valid_until     TEXT    DEFAULT NULL,
    supersedes_id   INTEGER DEFAULT NULL REFERENCES assertions(id),
    confidence      REAL    DEFAULT 1.0,
    asserted_by     TEXT    NOT NULL DEFAULT 'agent',
    source_ref      TEXT    NOT NULL DEFAULT '',
    evidence_text   TEXT    NOT NULL DEFAULT '',
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    CHECK (valid_from IS NULL OR valid_until IS NULL OR valid_until >= valid_from),
    CHECK (asserted_by IN ('agent', 'manual', 'import', 'enrichment'))
);

CREATE INDEX IF NOT EXISTS idx_assertions_subj ON assertions(subject);
CREATE INDEX IF NOT EXISTS idx_assertions_pred ON assertions(predicate);

-- ============================================================
-- tags
-- ============================================================
CREATE TABLE IF NOT EXISTS tags (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    tag     TEXT    NOT NULL,
    UNIQUE(page_id, tag)
);

CREATE INDEX IF NOT EXISTS idx_tags_tag     ON tags(tag);
CREATE INDEX IF NOT EXISTS idx_tags_page_id ON tags(page_id);

-- ============================================================
-- raw_data: sidecar data (replaces .raw/ JSON files)
-- ============================================================
CREATE TABLE IF NOT EXISTS raw_data (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id     INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    source      TEXT    NOT NULL,
    data        TEXT    NOT NULL,
    fetched_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(page_id, source)
);

CREATE INDEX IF NOT EXISTS idx_raw_data_page ON raw_data(page_id);

-- ============================================================
-- timeline_entries: structured timeline rows
-- ============================================================
CREATE TABLE IF NOT EXISTS timeline_entries (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id      INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    date         TEXT    NOT NULL,
    source       TEXT    NOT NULL DEFAULT '',
    summary      TEXT    NOT NULL,
    summary_hash TEXT    NOT NULL DEFAULT '',
    detail       TEXT    NOT NULL DEFAULT '',
    created_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(page_id, date, summary_hash)
);

CREATE INDEX IF NOT EXISTS idx_timeline_page ON timeline_entries(page_id);
CREATE INDEX IF NOT EXISTS idx_timeline_date ON timeline_entries(date);

-- ============================================================
-- raw_imports: original file bytes for byte-exact round-trip
-- ============================================================
CREATE TABLE IF NOT EXISTS raw_imports (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id    INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    import_id  TEXT    NOT NULL,
    is_active  INTEGER NOT NULL DEFAULT 1,
    raw_bytes  BLOB    NOT NULL,
    file_path  TEXT    NOT NULL,
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(page_id, import_id)
);

CREATE INDEX IF NOT EXISTS idx_raw_imports_page   ON raw_imports(page_id);
CREATE INDEX IF NOT EXISTS idx_raw_imports_active ON raw_imports(page_id, is_active)
    WHERE is_active = 1;

CREATE TABLE IF NOT EXISTS import_manifest (
    import_id   TEXT    PRIMARY KEY,
    source_dir  TEXT    NOT NULL,
    page_count  INTEGER NOT NULL,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- ============================================================
-- ingest_log: per-file SHA-256 idempotency audit trail.
-- Kept for compatibility with the import/ingest/embed commands;
-- will be removed when the reconciler slice replaces quaid import.
-- ============================================================
CREATE TABLE IF NOT EXISTS ingest_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    ingest_key   TEXT    NOT NULL UNIQUE,   -- SHA-256 of raw file bytes
    source_type  TEXT    NOT NULL,          -- 'file' | 'stdin'
    source_ref   TEXT    NOT NULL DEFAULT '',
    pages_updated TEXT   NOT NULL DEFAULT '[]',
    summary      TEXT    NOT NULL DEFAULT '',
    completed_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_ingest_log_key ON ingest_log(ingest_key);

-- ============================================================
-- file_state: stat-based change detection for vault sync
-- ============================================================
CREATE TABLE IF NOT EXISTS file_state (
    collection_id   INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    relative_path   TEXT    NOT NULL,
    page_id         INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    mtime_ns        INTEGER NOT NULL,
    ctime_ns        INTEGER DEFAULT NULL,
    size_bytes      INTEGER NOT NULL,
    inode           INTEGER DEFAULT NULL,
    sha256          TEXT    NOT NULL,
    last_seen_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    last_full_hash_at TEXT  NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (collection_id, relative_path)
);

CREATE INDEX IF NOT EXISTS idx_file_state_sha256 ON file_state(sha256);
CREATE INDEX IF NOT EXISTS idx_file_state_audit ON file_state(last_full_hash_at);
CREATE INDEX IF NOT EXISTS idx_file_state_page ON file_state(page_id);

-- ============================================================
-- embedding_jobs: persistent queue for async embedding work
-- ============================================================
CREATE TABLE IF NOT EXISTS embedding_jobs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id     INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL DEFAULT 0,
    priority    INTEGER NOT NULL DEFAULT 0,
    job_state   TEXT    NOT NULL DEFAULT 'pending'
                        CHECK(job_state IN ('pending', 'running', 'failed')),
    attempt_count INTEGER NOT NULL DEFAULT 0,
    last_error  TEXT    DEFAULT NULL,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    enqueued_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    started_at  TEXT    DEFAULT NULL,
    UNIQUE(page_id)
);

CREATE INDEX IF NOT EXISTS idx_embedding_jobs_queue
    ON embedding_jobs(job_state, priority DESC, enqueued_at);

-- ============================================================
-- config: mutable runtime defaults
-- ============================================================
CREATE TABLE IF NOT EXISTS config (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT OR IGNORE INTO config (key, value) VALUES
    ('version',               '7'),
    ('embedding_model',       'BAAI/bge-small-en-v1.5'),
    ('embedding_dimensions',  '384'),
    ('chunk_strategy',        'section'),
    ('search_merge_strategy', 'set-union'),
    ('default_token_budget',  '4000');

-- ============================================================
-- contradictions: detected inconsistencies
-- ============================================================
CREATE TABLE IF NOT EXISTS contradictions (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id       INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    other_page_id INTEGER REFERENCES pages(id) ON DELETE CASCADE,
    type          TEXT    NOT NULL,
    description   TEXT    NOT NULL,
    detected_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    resolved_at   TEXT    DEFAULT NULL
);

CREATE INDEX IF NOT EXISTS idx_contradictions_page       ON contradictions(page_id);
CREATE INDEX IF NOT EXISTS idx_contradictions_other_page ON contradictions(other_page_id);
CREATE INDEX IF NOT EXISTS idx_contradictions_unresolved ON contradictions(resolved_at)
    WHERE resolved_at IS NULL;

-- ============================================================
-- knowledge_gaps: queries the memory engine couldn't answer well
-- Privacy-safe by default: raw query text is NOT retained
-- unless explicitly approved.  Only query_hash is stored on
-- detection; query_text is populated post-approval.
-- ============================================================
CREATE TABLE IF NOT EXISTS knowledge_gaps (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id          INTEGER DEFAULT NULL REFERENCES pages(id) ON DELETE CASCADE,
    query_hash       TEXT    NOT NULL,   -- SHA-256 of original query, always stored
    query_text       TEXT    DEFAULT NULL,  -- raw text retained only after approval
    context          TEXT    NOT NULL DEFAULT '',
    confidence_score REAL    DEFAULT NULL,
    sensitivity      TEXT    NOT NULL DEFAULT 'internal',
    approved_by      TEXT    DEFAULT NULL,
    approved_at      TEXT    DEFAULT NULL,
    redacted_query   TEXT    DEFAULT NULL,
    resolved_at      TEXT    DEFAULT NULL,
    resolved_by_slug TEXT    DEFAULT NULL,
    detected_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    CHECK (sensitivity IN ('internal', 'external', 'redacted')),
    CHECK (query_text IS NULL OR (approved_by IS NOT NULL AND approved_at IS NOT NULL)),
    CHECK (sensitivity = 'internal' OR (approved_by IS NOT NULL AND approved_at IS NOT NULL))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_gaps_query_hash ON knowledge_gaps(query_hash);
CREATE INDEX IF NOT EXISTS idx_gaps_page ON knowledge_gaps(page_id) WHERE page_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_gaps_unresolved ON knowledge_gaps(resolved_at)
    WHERE resolved_at IS NULL;
